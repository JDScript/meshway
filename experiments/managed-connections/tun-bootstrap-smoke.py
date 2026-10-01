#!/usr/bin/env python3
"""Locked, controller-free TUN/capability proof; only uniquely owned scratch resources."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import secrets
import signal
import sys
import time

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("shell_smoke", HERE / "shell-smoke.py")
assert spec and spec.loader
shell = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shell)

# PID 1 retains a real, nonpersistent TUN FD. No enrolled state, grant or controller.
PROOF = r'''
import errno, fcntl, json, os, pathlib, signal, socket, stat, struct, subprocess, sys
uid, gid = map(int, sys.argv[1:3])
assert os.getpid() == 1 and uid > 0 and os.getuid() == uid and os.getgid() == gid and os.getgroups() == []
status = dict(line.split(":", 1) for line in pathlib.Path("/proc/self/status").read_text().splitlines())
caps = {key: status[key].strip() for key in ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")}
assert all(int(value, 16) == 0x1000 for value in caps.values()) and status["NoNewPrivs"].strip() == "1"
assert any(line.split()[1] == "/" and "ro" in line.split()[3].split(",") for line in pathlib.Path("/proc/mounts").read_text().splitlines())
try:
    pathlib.Path("/tmp/meshway-rootfs-write-must-fail").write_text("unexpected")
    raise AssertionError("rootfs was writable")
except OSError as error:
    assert error.errno == errno.EROFS
assert stat.S_IMODE(pathlib.Path("/state").stat().st_mode) == 0o700 and pathlib.Path("/state").stat().st_uid == uid
fd = os.open("/dev/net/tun", os.O_RDWR)
fcntl.ioctl(fd, 0x400454ca, struct.pack("16sH", b"meshway0", 0x1001))
def ip(*args):
    return subprocess.check_output(["ip", *args], text=True)
ip("-4", "addr", "add", "100.127.255.1/32", "dev", "meshway0")
ip("link", "set", "dev", "meshway0", "mtu", "1000", "up")
ip("-4", "route", "add", "unreachable", "100.127.255.0/24", "metric", "42760", "proto", "242")
ip("-4", "route", "add", "100.127.255.0/24", "dev", "meshway0", "src", "100.127.255.1", "metric", "42759", "proto", "242")
assert pathlib.Path("/proc/sys/net/ipv6/conf/default/disable_ipv6").read_text().strip() == "1"
assert all(not item.get("addr_info", []) for item in json.loads(ip("-j", "-6", "addr", "show", "dev", "meshway0")))
assert pathlib.Path("/proc/sys/net/ipv4/ip_forward").read_text().strip() == "0"
assert pathlib.Path("/proc/sys/net/ipv6/conf/all/forwarding").read_text().strip() == "0"
proof = {"uid": uid, "gid": gid, "pid": os.getpid(), "capabilities": caps, "no_new_privs": True,
    "rootfs": "read-only", "supplementary_groups": [], "state_mode": "0700",
    "addresses": json.loads(ip("-j", "-4", "addr", "show", "dev", "meshway0")),
    "routes": json.loads(ip("-j", "-N", "-4", "route", "show", "table", "main", "root", "100.127.255.0/24"))}
os.umask(0o077)
pathlib.Path("/state/proof.json").write_text(json.dumps(proof))
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
signal.pause()
'''

EXEC_PROOF = r'''
import json, os, pathlib, subprocess, sys
uid, gid = map(int, sys.argv[1:3])
assert os.getuid() == uid and os.getgid() == gid
status = dict(line.split(":", 1) for line in pathlib.Path("/proc/self/status").read_text().splitlines())
assert int(status["CapEff"], 16) == 0 and int(status["CapAmb"], 16) == 0
subprocess.run(["ping", "-c", "1", "-W", "2", "127.0.0.1"], check=True, capture_output=True, timeout=5)
proof = json.loads(pathlib.Path("/state/proof.json").read_text())
proof.update({"exec_uid": uid, "exec_gid": gid, "exec_effective_caps": 0, "unprivileged_ping": "127.0.0.1"})
print(json.dumps(proof))
'''


def smoke(image):
    uid, gid = os.getuid(), os.getgid()
    if uid == 0:
        raise RuntimeError("non-root host user required")
    info = json.loads(shell.docker("image", "inspect", image, "--format", "{{json .}}"))
    if info["Os"] != "linux" or not shell.native(platform.machine()) == shell.native(shell.docker("info", "--format", "{{.Architecture}}")) == shell.native(info["Architecture"]):
        raise RuntimeError("native Linux image/engine required")
    pin = info["Id"]
    run = secrets.token_hex(8)
    name = "meshway-tun-bootstrap-" + run
    labels = {"io.meshway.tun-bootstrap.run": run, "io.meshway.tun-bootstrap.project": "managed-neighbors"}
    args = [item for key, value in labels.items() for item in ("--label", key + "=" + value)]
    evidence = None
    try:
        shell.docker("create", "--name", name, "--pull", "never", *args, "--network", "none", "--user", "0:0",
                     "--cap-drop", "ALL", "--cap-add", "NET_ADMIN", "--cap-add", "SETUID", "--cap-add", "SETGID", "--cap-add", "SETPCAP",
                     "--device", "/dev/net/tun:/dev/net/tun:rw", "--security-opt", "no-new-privileges", "--read-only",
                     "--sysctl", "net.ipv4.ping_group_range=0 2147483647", "--sysctl", "net.ipv4.ip_forward=0",
                     "--sysctl", "net.ipv6.conf.all.forwarding=0", "--sysctl", "net.ipv6.conf.default.disable_ipv6=1",
                     "--pids-limit", "32", "--memory", "128m", "--log-driver", "none",
                     "--tmpfs", f"/state:rw,nosuid,nodev,noexec,mode=0700,uid={uid},gid={gid},size=1m",
                     "--entrypoint", "setpriv", pin, "--reuid", str(uid), "--regid", str(gid), "--clear-groups",
                     "--bounding-set=-all,+net_admin", "--inh-caps=-all,+net_admin", "--ambient-caps=-all,+net_admin", "--no-new-privs",
                     "python3", "-c", PROOF, str(uid), str(gid))
        shell.docker("start", name)
        for _ in range(30):
            try:
                evidence = json.loads(shell.docker("exec", "--user", f"{uid}:{gid}", name, "python3", "-c", EXEC_PROOF, str(uid), str(gid)))
                break
            except RuntimeError:
                if shell.docker("inspect", name, "--format", "{{.State.Running}}") != "true":
                    raise
                time.sleep(0.1)
        if evidence is None:
            raise RuntimeError("scratch TUN proof deadline exceeded")
    finally:
        if shell.docker("container", "ls", "--all", "--filter", "name=^/" + name + "$", "--format", "{{.Names}}"):
            owned = json.loads(shell.docker("container", "inspect", name, "--format", "{{json .}}"))
            if owned["Image"] != pin or any(owned["Config"]["Labels"].get(key) != value for key, value in labels.items()):
                raise RuntimeError("scratch ownership changed")
            shell.docker("rm", "--force", name)
    print(json.dumps({"result": "PASS", "image_id": pin, "architecture": info["Architecture"], "cleanup": "complete", **evidence}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    args = parser.parse_args()
    def interrupted(*_):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    with shell.performance_lock() as descriptor:
        shell.LOCK_FD = descriptor
        smoke(args.image)


if __name__ == "__main__":
    try:
        main()
    except (Exception, KeyboardInterrupt):
        print("TUN bootstrap smoke failed; scratch cleanup attempted; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
