#!/usr/bin/env python3
"""Locked host launcher and stdin-only, container-local enrollment helper."""

import hashlib
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import tempfile

DEFAULT_LOCK = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock"
# Local Agent disk sidecars, not HTTP DTOs. Never copy profiles, locks or IPC.
VERIFY_FILES = {
    "agent.json": (True, 256 * 1024),
    "agent.json.connection-key.json": (True, 8192),
    "agent.json.config.json": (False, 1024 * 1024 + 64 * 1024),
    "agent.json.connections.json": (False, 1024 * 1024 + 64 * 1024),
}


def checked(arguments):
    result = subprocess.run(arguments, capture_output=True, text=True, timeout=30, check=False)
    if result.returncode:
        raise RuntimeError("Agent command failed; private output suppressed")
    return result.stdout


def container_image_check(data):
    """Network-free validation; no state, credentials or controller mutation."""
    help_text = checked(["meshway-agent", "--help"])
    run_help = checked(["meshway-agent", "run", "--help"])
    if not all(flag in run_help for flag in ("--apply-connections", "--connection-profiles", "--connection-lease-seconds")):
        raise RuntimeError("managed Agent required")
    if data.get("dataplane", False):
        if not all(flag in run_help for flag in ("--apply-dataplane", "--tun-name", "--tun-mtu")):
            raise RuntimeError("image does not expose managed IPv4 TUN mode")
        checked(["setpriv", "--version"])
        checked(["ip", "-Version"])
    checked(["meshway-controller", "--help"])
    ping_cli = re.search(r"^\s+ping\s", help_text, re.MULTILINE) is not None
    if ping_cli:
        ping_help = checked(["meshway-agent", "ping", "--help"])
        if not all(flag in ping_help for flag in ("--count", "--timeout-seconds", "--state")):
            raise RuntimeError("unsupported ping CLI")
    if data["shell"]:
        alias = Path("/usr/local/bin/meshway")
        if data.get("development", False):
            if not alias.is_file() or alias.is_symlink():
                raise RuntimeError("developer meshway privilege-dropping launcher required")
            os_release = Path("/etc/os-release").read_text()
            if "ID=ubuntu" not in os_release or 'VERSION_ID="24.04"' not in os_release:
                raise RuntimeError("Ubuntu 24.04 developer runtime required")
            for command in ("apt-get", "bash", "curl", "ping", "ip", "python3", "setpriv", "traceroute"):
                checked(["sh", "-c", f"command -v {command}"])
        elif alias.resolve() != Path("/usr/local/bin/meshway-agent"):
            raise RuntimeError("meshway alias required")
        if os.environ.get("MESHWAY_STATE") != "/state/agent.json":
            raise RuntimeError("default state binding required")
        if "MESHWAY_STATE" not in checked(["meshway", "status", "--help"]):
            raise RuntimeError("state environment support required")
        checked(["ping", "-c", "1", "-W", "2", "127.0.0.1"])
    return {"managed": True, "ping_cli": ping_cli}


def private_source_bytes(directory_fd, name, required, limit):
    """Read only, relative to a pinned directory; reject link/special-file tricks."""
    try:
        descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory_fd)
    except FileNotFoundError:
        if not required:
            return None
        raise
    with os.fdopen(descriptor, "rb") as source:
        before = os.fstat(source.fileno())
        if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.getuid()
                or stat.S_IMODE(before.st_mode) != 0o600 or before.st_nlink != 1 or before.st_size > limit):
            raise RuntimeError("verification source must be a bounded private regular file without hardlinks")
        contents = source.read(limit + 1)
        after = os.fstat(source.fileno())
        # A running Agent may atomically replace an optional cache while this FD
        # is read. Its old, fully read inode remains valid; in-place edits do not.
        if (len(contents) != before.st_size or after.st_size != before.st_size
                or after.st_mtime_ns != before.st_mtime_ns or after.st_nlink > 1
                or after.st_mode != before.st_mode or after.st_uid != before.st_uid):
            raise RuntimeError("verification source changed while reading")
        return contents


def container_verify(data, source_path="/source-state", scratch_path="/state"):
    """Run real status CLIs on disposable private copies, never writable originals.

    The protocol's local reader opens files read+write even for status. The Docker
    adapter binds /source-state read-only and supplies a private writable tmpfs at
    /state, with --network none. No heartbeat/enroll/publish command is invoked.
    """
    os.umask(0o077)
    if (os.getuid() == 0 or data["operation"] != "verify" or data["token"] is not None
            or any(not isinstance(data[key], str) or re.fullmatch(r"[0-9a-f]{64}", data[key]) is None
                   for key in ("expected_id", "expected_key"))):
        raise RuntimeError("read-only verification requires original enrolled identity and key")
    descriptor = os.open(source_path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        directory = os.fstat(descriptor)
        scratch = os.lstat(scratch_path)
        if any(not stat.S_ISDIR(item.st_mode) or item.st_uid != os.getuid()
               or stat.S_IMODE(item.st_mode) != 0o700 for item in (directory, scratch)):
            raise RuntimeError("verification directories must be private and user-owned")
        with tempfile.TemporaryDirectory(prefix="verify-", dir=scratch_path) as temporary:
            immutable = {}
            for name, (required, limit) in VERIFY_FILES.items():
                contents = private_source_bytes(descriptor, name, required, limit)
                if contents is None:
                    continue
                destination = Path(temporary) / name
                with destination.open("xb") as copy:
                    copy.write(contents)
                digest = hashlib.sha256(contents).digest()
                if hashlib.sha256(destination.read_bytes()).digest() != digest:
                    raise RuntimeError("verification copy differs from source bytes")
                if required:
                    immutable[name] = digest
            result = container_enroll(data, str(Path(temporary) / "agent.json"))
            for name, digest in immutable.items():
                required, limit = VERIFY_FILES[name]
                if hashlib.sha256(private_source_bytes(descriptor, name, required, limit)).digest() != digest:
                    raise RuntimeError("original identity/key changed during verification")
            return result
    finally:
        os.close(descriptor)


def running_exec_user(uid, gid):
    """The Docker exec process must be the original user with zero capabilities."""
    if (type(uid) is not int or type(gid) is not int or not 0 < uid < 2**32 or not 0 <= gid < 2**32
            or (os.getuid(), os.geteuid(), os.getgid(), os.getegid()) != (uid, uid, gid, gid)):
        raise RuntimeError("original non-root execution identity required")
    status = dict(line.split(":", 1) for line in Path("/proc/self/status").read_text().splitlines())
    if (any(int(status[key], 16) != 0 for key in ("CapEff", "CapPrm", "CapAmb"))
            or status["NoNewPrivs"].strip() != "1"):
        raise RuntimeError("verification exec must have zero effective/permitted/ambient capabilities and no-new-privileges")
    if not any(line.split()[1] == "/" and "ro" in line.split()[3].split(",")
               for line in Path("/proc/mounts").read_text().splitlines()):
        raise RuntimeError("original image root filesystem must remain read-only")


def running_agent_architecture():
    # Fixed installed executable on the validated read-only rootfs. This is a
    # bounded ELF header read, not an inference from the container's kernel uname.
    with open("/usr/local/bin/meshway-agent", "rb") as binary:
        header = binary.read(64)
    if len(header) != 64 or header[:6] != b"\x7fELF\x02\x01":
        raise RuntimeError("native 64-bit little-endian Agent ELF required")
    architecture = {183: "arm64", 62: "amd64"}.get(int.from_bytes(header[18:20], "little"))
    if architecture is None:
        raise RuntimeError("unsupported Agent ELF architecture")
    return architecture


def container_running_check(data):
    """Old installed CLI verification; original /state opens are O_RDONLY only."""
    if not isinstance(data, dict) or set(data) != {"uid", "gid", "dataplane", "verify"} or type(data["dataplane"]) is not bool:
        raise RuntimeError("invalid public running-image request")
    verify = data["verify"]
    if (not isinstance(verify, dict) or set(verify) != {"operation", "name", "controller", "token", "expected_id", "expected_key"}
            or verify["operation"] != "verify" or verify["token"] is not None
            or verify["name"] not in ("demo-nyc", "demo-lax", "demo-sha")
            or not isinstance(verify["controller"], str) or len(verify["controller"]) > 2048
            or re.fullmatch(r"https?://[^\s@?#]+", verify["controller"]) is None
            or any(not isinstance(verify[key], str) or re.fullmatch(r"[0-9a-f]{64}", verify[key]) is None
                   for key in ("expected_id", "expected_key"))):
        raise RuntimeError("only public original binding and read-only verify are allowed")
    running_exec_user(data["uid"], data["gid"])
    architecture = running_agent_architecture()
    image = container_image_check({"shell": False, "dataplane": data["dataplane"]})
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix="meshway-old-verify-", dir="/tmp") as scratch:
        result = container_verify(verify, source_path="/state", scratch_path=scratch)
    return {**image, **result, "architecture": architecture}


def container_enroll(data, state_path="/state/agent.json"):
    """Never mint a replacement credential for an existing state file.

    The original grant arrives on stdin, not host Docker argv. The real Agent
    persists the proposed credential before enrollment. Heartbeat can recover a
    committed enrollment whose response was lost, even after the grant expires.
    """
    os.umask(0o077)
    if os.getuid() == 0:
        raise RuntimeError("non-root Agent required")
    state = Path(state_path)
    name, controller, token = data["name"], data["controller"], data["token"]
    if name not in ("demo-nyc", "demo-lax", "demo-sha") or not isinstance(controller, str):
        raise RuntimeError("invalid binding")
    if token is not None and (not isinstance(token, str) or re.fullmatch(r"[0-9a-f]{64}", token) is None):
        raise RuntimeError("invalid original grant")

    def status():
        value = json.loads(checked(["meshway-agent", "status", "--state", str(state)]))
        if value["name"] != name or value["controller"] != controller:
            raise RuntimeError("different immutable binding")
        node_id = value["node_id"]
        if node_id is not None and re.fullmatch(r"[0-9a-f]{64}", node_id) is None:
            raise RuntimeError("invalid identity")
        if data["expected_id"] is not None and node_id != data["expected_id"]:
            raise RuntimeError("identity differs from demo metadata")
        return node_id

    if data["operation"] in ("publish", "verify"):
        node_id = status()
        if node_id is None:
            raise RuntimeError("cannot publish a pending identity")
        connection = json.loads(checked(["meshway-agent", "connections", "status", "--state", str(state)]))
        expected_key = data["expected_key"]
        if expected_key is not None and connection["public_key"] != expected_key:
            raise RuntimeError("bound connection key missing or changed; refusing regeneration")
        if data["operation"] == "publish":
            checked(["meshway-agent", "connections", "publish", "--state", str(state)])
        return {"node_id": node_id}
    if data["operation"] != "recover":
        raise RuntimeError("invalid helper operation")
    if state.exists() or state.is_symlink():
        before = status()  # Real CLI validates private state, not just JSON shape.
        try:
            checked(["meshway-agent", "heartbeat", "--state", str(state)])
        except Exception:
            # An enrolled/revoked/unrecognized identity never falls back to enroll.
            if before is not None or token is None:
                raise
            checked(["meshway-agent", "enroll", "--controller", controller,
                     "--token", token, "--name", name, "--state", str(state)])
    else:
        if token is None or data["expected_id"] is not None:
            raise RuntimeError("missing original enrollment state")
        checked(["meshway-agent", "enroll", "--controller", controller,
                 "--token", token, "--name", name, "--state", str(state)])
    node_id = status()
    if node_id is None:
        raise RuntimeError("enrollment is still pending")
    return {"node_id": node_id}


def main():
    if sys.argv[1:2] == ["--container-verify"]:
        try:
            print(json.dumps(container_verify(json.load(sys.stdin))))
            return 0
        except Exception:
            print("private-copy verification failed; private diagnostics suppressed", file=sys.stderr)
            return 1
    if sys.argv[1:2] == ["--container-image-check"]:
        try:
            print(json.dumps(container_image_check(json.load(sys.stdin))))
            return 0
        except Exception:
            print("image validation failed; private diagnostics suppressed", file=sys.stderr)
            return 1
    if sys.argv[1:2] == ["--container-enroll"]:
        try:
            print(json.dumps(container_enroll(json.load(sys.stdin))))
            return 0
        except Exception:
            print("demo enrollment failed; private diagnostics suppressed", file=sys.stderr)
            return 1

    here = Path(__file__).resolve().parent
    if "--help" in sys.argv[1:]:
        return subprocess.run(["node", str(here / "demo.ts"), "--help"], check=False).returncode
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(here.parent / "transport"))
    from performance_lock import performance_lock

    os.environ.setdefault("MESHWAY_PERF_LOCK", DEFAULT_LOCK)
    with performance_lock() as descriptor:
        child = subprocess.Popen(
            ["node", str(here / "demo.ts"), *sys.argv[1:]],
            env={**os.environ, "MESHWAY_DEMO_LOCK_FD": str(descriptor)},
            pass_fds=(descriptor,),
        )
        while True:
            try:
                return child.wait()
            except KeyboardInterrupt:
                # Node finishes its bounded command and stops owned containers;
                # keep the inherited lock alive throughout that cleanup.
                child.send_signal(signal.SIGINT)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        print("demo launcher failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
