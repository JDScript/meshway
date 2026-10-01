#!/usr/bin/env python3
"""Extend the existing private identity helper; no Controller HTTP implementation."""
import hashlib
import ipaddress
import json
from pathlib import Path
import re
import subprocess
import sys

sys.dont_write_bytecode = True
import base


def child(command):
    result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=8)
    try:
        output = json.loads(result.stdout)
    except ValueError:
        output = None
    return {"exit": result.returncode, "output": output}


def udp_inventory(agent_pid):
    """Match process socket FD inodes against /proc UDP tables (not global ports)."""
    if agent_pid is None:
        return []
    inodes = {}
    try:
        descriptors = list(Path(f"/proc/{agent_pid}/fd").iterdir())
    except FileNotFoundError:
        return []
    for fd in descriptors:
        try:
            match = re.fullmatch(r"socket:\[(\d+)\]", str(fd.readlink()))
            if match:
                inodes[match[1]] = inodes.get(match[1], 0) + 1
        except FileNotFoundError:
            continue
    sockets = []
    for family, filename in ((4, "udp"), (6, "udp6")):
        try:
            content = Path(f"/proc/{agent_pid}/net/{filename}").read_text()
        except FileNotFoundError:
            return []
        sockets.extend(parse_udp_table(content, inodes, family))
    return sockets


def parse_udp_table(content, inodes, family):
    sockets = []
    for line in content.splitlines()[1:]:
        fields = line.split()
        if fields[9] not in inodes:
            continue
        address, port = fields[1].split(":")
        raw = bytes.fromhex(address)
        raw = b"".join(raw[index:index + 4][::-1] for index in range(0, len(raw), 4))
        sockets.append({"family": family, "host": str(ipaddress.ip_address(raw)),
                        "port": int(port, 16), "inode": fields[9], "fd_count": inodes[fields[9]]})
    return sockets


def main(action, data):
    if action == "agent-start":
        command = ["meshway-agent", "run", "--state", str(base.STATE), "--apply-connections",
                   "--interval-seconds", "1", "--connection-lease-seconds", "10"]
        if data.get("dns") is True:
            command.extend(["--dns-listen", "127.0.0.1:5353"])
        if data.get("observer") is not None:
            address = ipaddress.IPv4Address(data["observer"])
            if not address.is_private or address.is_loopback or address.is_unspecified:
                raise ValueError("invalid private observer")
            command.extend(["--stun-server", f"{address}:3478"])
        return base.start("agent", command)
    if action == "observe":
        value = base.observe()
        value["udp"] = udp_inventory(value["agent_pid"])
        if base.pid("agent") is None:
            value.update(agent_pid=None, agent_instances=0, udp=[])
        # Public proof of exact private bytes across restarts, never the bytes.
        value["identity_digest"] = hashlib.sha256(base.STATE.read_bytes()).hexdigest() if base.STATE.exists() else None
        key = Path(str(base.STATE) + ".connection-key.json")
        value["key_digest"] = hashlib.sha256(key.read_bytes()).hexdigest() if key.exists() else None
        value["ss_udp"] = base.checked(["ss", "-H", "-lnu"])
        return value
    if action == "live":
        return child(["meshway-agent", "status", "--state", str(base.STATE), "--live", "--peers", "--json"])
    if action == "ping":
        peer = data.get("node_id", "")
        if re.fullmatch(r"[0-9a-f]{64}", peer) is None:
            raise ValueError("invalid peer")
        return child(["meshway-agent", "ping", peer, "--state", str(base.STATE), "--count", "1",
                      "--timeout-seconds", "2", "--json"])
    if action == "observer-start":
        return base.start("observer", ["python3", "/private/observer.py", json.dumps(data)])
    if action == "observer-stop":
        return base.stop("observer")
    if action == "observer-state":
        path = Path("/private/observer.json")
        return json.loads(path.read_text()) if path.exists() else {"count": 0, "peers": []}
    if action == "pause":
        import os
        import signal
        value = base.pid("agent")
        if value is None:
            raise ValueError("Agent absent")
        os.kill(value, signal.SIGSTOP if data.get("paused") is True else signal.SIGCONT)
        return {"paused": data.get("paused") is True}
    if action == "tools":
        help_text = base.checked(["meshway-agent", "run", "--help"])
        if any(flag not in help_text for flag in ("--apply-connections", "--stun-server", "--connection-lease-seconds")):
            raise ValueError("image requires actual-carrier discovery")
        base.checked(["ss", "--version"])
        return {"ready": True}
    return base.main(action, data)


if __name__ == "__main__":
    try:
        print(json.dumps(main(sys.argv[1], json.load(sys.stdin))))
    except Exception:
        print("managed discovery helper failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
