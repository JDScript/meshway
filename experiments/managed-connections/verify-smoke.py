#!/usr/bin/env python3
"""Network-free real-CLI verification-copy regression; synthetic state only."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import secrets
import signal
import sys
import tempfile

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("meshway_shell_smoke", HERE / "shell-smoke.py")
assert spec and spec.loader
shell = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shell)

# RFC 7748 test vector. This public test key and synthetic identity are never
# enrolled, published, or sent to a controller. Containers have --network none.
PRIVATE_KEY = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"
PUBLIC_KEY = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"


def fingerprint(source):
    return {path.name: (hashlib.sha256(path.read_bytes()).digest(), path.stat().st_ino,
                        path.stat().st_mode, path.stat().st_mtime_ns) for path in source.iterdir()}


def smoke(image, source, expected):
    docker = shell.docker
    uid, gid = os.getuid(), os.getgid()
    if uid == 0:
        raise RuntimeError("non-root host user required")
    info = json.loads(docker("image", "inspect", image, "--format", "{{json .}}"))
    if info["Os"] != "linux" or not shell.native(platform.machine()) == shell.native(docker("info", "--format", "{{.Architecture}}")) == shell.native(info["Architecture"]):
        raise RuntimeError("native Linux image and engine required")
    pin = info["Id"]
    run = secrets.token_hex(8)
    name = "meshway-verify-" + run
    label = "io.meshway.verify-smoke.run"
    before = fingerprint(source)
    try:
        docker("create", "--name", name, "--label", label + "=" + run, "--pull", "never", "--network", "none",
               "--user", f"{uid}:{gid}", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--read-only",
               "--pids-limit", "32", "--memory", "128m", "--log-driver", "none",
               "--mount", f"type=bind,src={source},dst=/source-state,readonly",
               "--mount", f"type=bind,src={HERE / 'demo.py'},dst=/opt/meshway-demo.py,readonly",
               "--tmpfs", f"/state:rw,nosuid,nodev,noexec,mode=0700,uid={uid},gid={gid},size=8m",
               "--entrypoint", "sleep", pin, "infinity")
        docker("start", name)
        docker("exec", name, "python3", "-c", """
import errno, os
try:
    descriptor = os.open('/source-state/agent.json', os.O_RDWR)
except OSError as error:
    assert error.errno == errno.EROFS
else:
    os.close(descriptor)
    raise RuntimeError('source mount unexpectedly writable')
""")
        result = json.loads(docker("exec", "--interactive", name, "python3", "/opt/meshway-demo.py", "--container-verify", input=json.dumps(expected)))
        if result != {"node_id": expected["expected_id"]}:
            raise RuntimeError("real CLI failed to validate synthetic identity/key")
        docker("exec", name, "python3", "-c", "import os; assert os.listdir('/state') == []")
        if fingerprint(source) != before:
            raise RuntimeError("original source changed")
    finally:
        if docker("container", "ls", "--all", "--filter", "name=^/" + name + "$", "--format", "{{.Names}}"):
            inspected = json.loads(docker("container", "inspect", name, "--format", "{{json .}}"))
            if inspected["Image"] != pin or inspected["Config"]["Labels"].get(label) != run:
                raise RuntimeError("scratch ownership mismatch; refusing cleanup")
            docker("rm", "--force", name)
    print(json.dumps({"result": "PASS", "image_id": pin, "uid": uid, "gid": gid,
                      "source_bind": "read-only", "verification": "real status + connections status on private tmpfs copies",
                      "original_hashes_and_metadata": "unchanged", "copies": "discarded", "network": "none", "cleanup": "complete"}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", action="append", required=True, help="prebuilt native image; repeat for old and new pins")
    args = parser.parse_args()
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    os.umask(0o077)
    parent = Path(tempfile.gettempdir()) / "opencode"
    parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="meshway-verify-", dir=parent) as root:
        source = Path(root).resolve()
        node_id = secrets.token_hex(32)
        controller = "http://127.0.0.1:9"
        identity = {"format_version": 1, "controller": controller, "name": "demo-nyc", "enrollment_version": "test",
                    "enrollment_token_hash": [0] * 32, "node_credential": secrets.token_hex(32), "node_id": node_id}
        key = {"format_version": 1, "controller": controller, "node_id": node_id, "private_key": PRIVATE_KEY, "public_key": PUBLIC_KEY}
        for filename, content in (("agent.json", identity), ("agent.json.connection-key.json", key)):
            (source / filename).write_text(json.dumps(content))
        expected = {"operation": "verify", "name": "demo-nyc", "controller": controller, "token": None,
                    "expected_id": node_id, "expected_key": PUBLIC_KEY}
        with shell.performance_lock() as descriptor:
            shell.LOCK_FD = descriptor
            for image in args.image:
                smoke(image, source, expected)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("verification-copy smoke interrupted; scratch cleanup attempted", file=sys.stderr)
        sys.exit(130)
    except Exception:
        print("verification-copy smoke failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
