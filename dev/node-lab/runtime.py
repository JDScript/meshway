#!/usr/bin/env python3
"""Container owner: private /etc, system bus, resolved, then meshwayd.

DNS execution plans and down/recovery belong exclusively to the real daemon.
This owner supplies OS services and stops in reverse dependency order.
"""
import ipaddress
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
import uuid

RUN = Path("/run/meshway-lab")
stopping = False


def command(*args):
    return subprocess.check_output(args, text=True, timeout=5).strip()


def stop_requested(_signum, _frame):
    global stopping
    stopping = True


def namespace():
    mount = os.readlink("/proc/self/ns/mnt")
    network = os.readlink("/proc/self/ns/net")
    if (os.getpid() != 1 or os.getuid() != 0 or mount == os.environ["MESHWAY_LAB_PARENT_MNT"]
            or network != os.environ["MESHWAY_LAB_PARENT_NET"]):
        raise RuntimeError("Expected the owned container's private mount namespace.")
    command("mount", "--make-rprivate", "/")
    if command("findmnt", "-n", "-o", "FSTYPE", "-T", "/run") != "tmpfs":
        raise RuntimeError("OS services require a container-private /run tmpfs.")
    if any(field.startswith(("shared:", "master:")) for line in Path("/proc/self/mountinfo").read_text().splitlines()
           for field in line.split(" - ")[0].split()[6:]):
        raise RuntimeError("Mount propagation is not private.")
    RUN.mkdir(mode=0o755, exist_ok=True)
    # Persist copied image /etc in this writable layer, separate from identity.
    # Docker bind files are only read as copy inputs, then detached and hidden.
    directory = Path("/var/lib/meshway-lab/etc")
    directory.parent.mkdir(mode=0o700, exist_ok=True)
    if not directory.exists():
        temporary = directory.with_name("etc.preparing")
        if temporary.exists():
            shutil.rmtree(temporary)
        command("cp", "-a", "/etc", str(temporary))
        temporary.rename(directory)
    for name in ("hostname", "hosts", "resolv.conf"):
        command("umount", "/etc/" + name)
    command("mount", "--bind", str(directory), "/etc")
    Path("/etc/machine-id").write_text(uuid.uuid4().hex + "\n")
    os.chmod("/etc/machine-id", 0o444)
    upstreams = os.environ.get("MESHWAY_LAB_UPSTREAM_DNS", "1.1.1.1 8.8.8.8").split()
    if not upstreams or len(upstreams) > 3:
        raise RuntimeError("Expected one to three explicit namespace upstream IPs.")
    for address in upstreams:
        parsed = ipaddress.ip_address(address)
        if parsed.is_loopback or parsed.is_unspecified or parsed.is_multicast:
            raise RuntimeError("Namespace upstream must be a real unicast IP.")
    # Underlying system baseline, independent of daemon bootstrap and Server plan.
    Path("/etc/systemd/resolved.conf").write_text("[Resolve]\nDNS=" + " ".join(upstreams) + "\n"
        "FallbackDNS=\nLLMNR=no\nMulticastDNS=no\nDNSStubListener=yes\n")
    resolver = Path("/etc/resolv.conf")
    resolver.unlink(missing_ok=True)
    resolver.symlink_to("/run/systemd/resolve/stub-resolv.conf")
    for path in ("/run/dbus", "/run/systemd"):
        Path(path).mkdir(parents=True, exist_ok=True)
    Path("/run/dbus/system_bus_socket").unlink(missing_ok=True)
    (RUN / "namespace.json").write_text(json.dumps({"parentMount": os.environ["MESHWAY_LAB_PARENT_MNT"],
        "mount": mount, "network": network, "etcMount": command("findmnt", "-n", "-o", "SOURCE,PROPAGATION", "-T", "/etc")}) + "\n")


def ready(children, check):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if stopping or any(child.poll() is not None for child in children):
            raise RuntimeError("Required service exited during startup.")
        try:
            if check():
                return
        except (subprocess.SubprocessError, OSError):
            pass
        time.sleep(.1)
    raise RuntimeError("Required service startup timed out.")


def resolved_ready(pid):
    # Never ask D-Bus to auto-activate a second, unowned service during startup.
    bus = ("busctl", "--system", "--auto-start=no")
    call = (*bus, "call", "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus")
    if command(*call, "NameHasOwner", "s", "org.freedesktop.resolve1") != "b true":
        return False
    if command(*call, "GetConnectionUnixProcessID", "s", "org.freedesktop.resolve1") != f"u {pid}":
        raise RuntimeError("resolve1 name is owned by a different process.")
    for name, value in (("ResolvConfMode", "stub"), ("DNSStubListener", "yes")):
        if command(*bus, "get-property", "org.freedesktop.resolve1", "/org/freedesktop/resolve1",
                   "org.freedesktop.resolve1.Manager", name) != f's "{value}"':
            raise RuntimeError("resolved does not provide the required dynamic stub contract.")
    return "nameserver 127.0.0.53" in Path("/etc/resolv.conf").read_text()


def stop_child(child):
    if child.poll() is None:
        child.terminate()
    try:
        child.wait(timeout=10)
        return True
    except subprocess.TimeoutExpired:
        child.kill()
        child.wait(timeout=3)
        return False


def start(*args):
    # Mount setup is finished. Services and Node never inherit SYS_ADMIN.
    return subprocess.Popen(["setpriv", "--bounding-set=-sys_admin", "--", *args])


def main():
    signal.signal(signal.SIGTERM, stop_requested)
    signal.signal(signal.SIGINT, stop_requested)
    namespace()
    state = Path("/var/lib/meshway")
    if state.is_symlink() or state.stat().st_uid != os.getuid():
        raise RuntimeError("Node state owner differs; refusing to repair identity ownership.")
    state.chmod(0o700)
    children = []
    status = 1
    try:
        bus = start("dbus-daemon", "--system", "--nofork", "--nopidfile")
        children.append(bus)
        ready(children, lambda: Path("/run/dbus/system_bus_socket").is_socket())
        resolved = start("/usr/lib/systemd/systemd-resolved")
        children.append(resolved)
        ready(children, lambda: resolved_ready(resolved.pid))
        daemon = start("/opt/meshway/meshwayd", *sys.argv[1:])
        children.append(daemon)
        (RUN / "services.json").write_text(json.dumps({"bus": bus.pid, "resolved": resolved.pid, "daemon": daemon.pid}) + "\n")
        while not stopping and all(child.poll() is None for child in children):
            time.sleep(.1)
        status = 0 if stopping else daemon.returncode if daemon.returncode is not None else 1
    finally:
        clean = True
        for child in reversed(children):
            clean = stop_child(child) and clean
        (RUN / "services.json").unlink(missing_ok=True)
        while True:
            try:
                pid, _ = os.waitpid(-1, os.WNOHANG)
                if pid == 0:
                    break
            except ChildProcessError:
                break
        if not clean:
            status = 1
    return status


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"Node lab OS services failed: {error}", file=sys.stderr)
        sys.exit(1)
