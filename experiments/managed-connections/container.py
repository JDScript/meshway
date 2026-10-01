#!/usr/bin/env python3
"""Private container process helper. No HTTP implementation or lab static config.

All input is stdin JSON. Child output is captured, never relayed on failure.
Only explicit public observations are returned; secret reads have dedicated verbs.
Docker's --init reaps detached children after this helper exits.
"""

import ipaddress
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import time

ROOT = Path("/private")
STATE = ROOT / "agent.json"
os.umask(0o077)


def checked(command, timeout=20):
    result = subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False)
    if result.returncode:
        raise RuntimeError("child command failed (output suppressed)")
    return result.stdout


def pid(name):
    path = ROOT / f"{name}.pid"
    if not path.exists():
        return None
    value = int(path.read_text())
    try:
        os.kill(value, 0)
    except ProcessLookupError:
        return None
    # Exited processes awaiting the container init's reap are not running.
    stat = Path(f"/proc/{value}/stat")
    if stat.exists() and stat.read_text().split(") ", 1)[1].startswith("Z "):
        return None
    return value


def start(name, command):
    if pid(name) is not None:
        raise RuntimeError("process already running")
    with (ROOT / f"{name}.log").open("ab") as log:
        child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
    (ROOT / f"{name}.pid").write_text(str(child.pid))
    return {"started": True}


def stop(name):
    value = pid(name)
    if value is not None:
        os.kill(value, signal.SIGINT)
        deadline = time.monotonic() + 6
        while time.monotonic() < deadline and pid(name) is not None:
            time.sleep(0.1)
        if pid(name) is not None:
            raise RuntimeError("process failed to shut down within deadline")
    return {"stopped": True}


def identity():
    value = json.loads(STATE.read_text())
    node_id = value.get("node_id")
    if not isinstance(node_id, str) or re.fullmatch(r"[0-9a-f]{64}", node_id) is None:
        raise RuntimeError("agent identity is not enrolled")
    return value


def endpoint(value):
    host, port = value.rsplit(":", 1)
    return host.strip("[]"), int(port)


def sockets():
    listeners = []
    for protocol, flag in (("tcp", "-lnt"), ("udp", "-lnu")):
        for line in checked(["ss", "-H", flag]).splitlines():
            host, port = endpoint(line.split()[-2])
            listeners.append({"protocol": protocol, "host": host, "port": port})
    established = []
    for line in checked(["ss", "-H", "-tn", "state", "established"]).splitlines():
        local_host, local_port = endpoint(line.split()[-2])
        peer_host, peer_port = endpoint(line.split()[-1])
        established.append({"local_host": local_host, "local_port": local_port, "peer_host": peer_host, "peer_port": peer_port})
    agent_pid = pid("agent")
    agent_inodes = set()
    if agent_pid is not None:
        for descriptor in Path(f"/proc/{agent_pid}/fd").iterdir():
            try:
                target = os.readlink(descriptor)
            except FileNotFoundError:
                continue
            if target.startswith("socket:["):
                agent_inodes.add(target[8:-1])
    udp_ports = []
    for table in ("udp", "udp6"):
        for line in Path(f"/proc/net/{table}").read_text().splitlines()[1:]:
            fields = line.split()
            if fields[9] in agent_inodes:
                udp_ports.append(int(fields[1].rsplit(":", 1)[1], 16))
    return {
        "listeners": listeners,
        "established": established,
        "agent_running": agent_pid is not None,
        "agent_udp_ports": sorted(set(udp_ports)),
        "controller_running": pid("controller") is not None,
        "ipv4_forwarding": Path("/proc/sys/net/ipv4/ip_forward").read_text().strip() == "1",
        "ipv6_forwarding": Path("/proc/sys/net/ipv6/conf/all/forwarding").read_text().strip() == "1",
        "interfaces": sorted(path.name for path in Path("/sys/class/net").iterdir()),
    }


def main(action, data):
    ROOT.mkdir(mode=0o700, exist_ok=True)
    if action == "controller-start":
        return start("controller", ["meshway-controller", "serve", "--bind", "0.0.0.0:8420", "--data-dir", str(ROOT / "controller")])
    if action == "controller-stop":
        return stop("controller")
    if action == "admin-token":
        return {"token": checked(["meshway-controller", "admin-token", "--data-dir", str(ROOT / "controller")]).strip()}
    if action == "enroll":
        if STATE.exists():
            raise RuntimeError("refusing to replace an existing identity")
        if data.get("name") not in ("NYC", "LAX", "SHA") or not isinstance(data.get("token"), str):
            raise RuntimeError("invalid enrollment input")
        # Current Agent CLI accepts --token only. The token enters this helper on
        # stdin and never appears in host Docker argv, diagnostics, or artifacts.
        checked(["meshway-agent", "enroll", "--controller", "http://lax.mesh.test:8420", "--token", data["token"], "--name", data["name"], "--state", str(STATE)], timeout=30)
        return {"node_id": identity()["node_id"]}
    if action == "node-bearer":
        return {"token": identity()["node_credential"]}
    if action == "identity":
        return {"node_id": identity()["node_id"]}
    if action == "agent-start":
        lease = data.get("lease_seconds")
        if not isinstance(lease, int) or not 10 <= lease <= 45:
            raise RuntimeError("invalid test lease")
        return start("agent", ["meshway-agent", "run", "--state", str(STATE), "--apply-connections", "--connection-profiles", str(ROOT / "profiles.json"), "--interval-seconds", "1", "--connection-lease-seconds", str(lease)])
    if action == "agent-stop":
        return stop("agent")
    if action == "install-files":
        files = data.get("files")
        if not isinstance(files, list) or len(files) > 4:
            raise RuntimeError("invalid file transfer")
        for item in files:
            if not isinstance(item, dict) or item.get("name") not in ("profiles.json", "lax-cert.pem", "lax-key.pem", "ca.pem") or not isinstance(item.get("content"), str) or len(item["content"]) > 256 * 1024:
                raise RuntimeError("invalid private file")
            path = ROOT / item["name"]
            temporary = path.with_suffix(path.suffix + ".new")
            with temporary.open("w") as output:
                output.write(item["content"])
            temporary.chmod(0o600)
            temporary.replace(path)
        return {"installed": True}
    if action == "sockets":
        return sockets()
    if action == "resolve":
        return {"addresses": sorted({entry[4][0] for entry in socket.getaddrinfo("lax.mesh.test", 8420, family=socket.AF_INET, type=socket.SOCK_STREAM)})}
    if action == "underlay-start":
        # An independently reachable TCP canary makes isolation negatives useful.
        return start("canary", ["python3", __file__, "canary"])
    if action == "underlay-probe":
        address = str(ipaddress.IPv4Address(data["address"]))
        try:
            with socket.create_connection((address, 42000), timeout=2):
                reachable = True
        except OSError:
            reachable = False
        return {"reachable": reachable}
    raise RuntimeError("unknown helper action")


if __name__ == "__main__":
    try:
        action = sys.argv[1]
        if action == "canary":
            with socket.socket() as server:
                server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                server.bind(("0.0.0.0", 42000))
                server.listen(8)
                while True:
                    connection, _ = server.accept()
                    connection.close()
        else:
            print(json.dumps(main(action, json.load(sys.stdin))))
    except Exception:
        # Deliberately exclude exception text: CLI arguments and private JSON can
        # otherwise be included by Python/subprocess/JSON exception formatting.
        print("managed test helper failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
