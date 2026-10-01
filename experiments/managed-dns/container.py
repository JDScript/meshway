#!/usr/bin/env python3
"""Private isolated process fixture. Controller HTTP always uses the generated host SDK."""
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import signal
import subprocess
import sys
import time

sys.dont_write_bytecode = True
from wire import query

ROOT = Path("/private")
STATE = ROOT / "agent.json"
VERSION = "managed-dns-acceptance-v1"
CONTROLLER = "http://controller.mesh.test:8420"
os.umask(0o077)


def checked(command):
    value = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20)
    if value.returncode:
        raise RuntimeError("child command failed; output suppressed")
    return value.stdout


def atomic(path, value):
    temporary = path.with_suffix(path.suffix + ".new")
    with temporary.open("x") as output:
        output.write(json.dumps(value))
        output.flush()
        os.fsync(output.fileno())
    temporary.chmod(0o600)
    temporary.replace(path)
    descriptor = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def pid(name):
    path = ROOT / f"{name}.pid"
    if not path.exists():
        return None
    value = int(path.read_text())
    try:
        os.kill(value, 0)
    except ProcessLookupError:
        return None
    stat = Path(f"/proc/{value}/stat")
    if stat.exists() and stat.read_text().split(") ", 1)[1].startswith("Z "):
        return None
    return value


def start(name, command):
    if pid(name) is not None:
        raise RuntimeError("process already running")
    with (ROOT / f"{name}.log").open("wb") as output:
        child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=output, stderr=output, start_new_session=True)
    (ROOT / f"{name}.pid").write_text(str(child.pid))
    return {"started": True}


def stop(name):
    value = pid(name)
    if value is not None:
        os.kill(value, signal.SIGINT)
        deadline = time.monotonic() + 6
        while pid(name) is not None and time.monotonic() < deadline:
            time.sleep(0.1)
        if pid(name) is not None:
            raise RuntimeError("process did not exit")
    return {"stopped": True}


def observe():
    agent_pid = pid("agent")
    processes = []
    for path in Path("/proc").glob("[0-9]*/comm"):
        try:
            if path.read_text().strip() == "meshway-agent":
                stat = (path.parent / "stat").read_text().split(") ", 1)[1]
                if not stat.startswith("Z "):
                    processes.append(int(path.parent.name))
        except FileNotFoundError:
            continue
    listeners = []
    for protocol, args in (("tcp", "-lnt"), ("udp", "-lnu")):
        for line in checked(["ss", "-H", args]).splitlines():
            host, port = line.split()[-2].rsplit(":", 1)
            if int(port) in (5353, 5354):
                listeners.append({"protocol": protocol, "host": host, "port": int(port)})
    links = json.loads(checked(["ip", "-j", "address", "show"]))
    tun = [{"name": link["ifname"], "up": "UP" in link["flags"],
            "ipv4": [entry["local"] for entry in link["addr_info"] if entry["family"] == "inet"]}
           for link in links if link["ifname"] == "meshway0"]
    proc = dict(line.split(":", 1) for line in Path("/proc/self/status").read_text().splitlines() if ":" in line)
    return {"agent_pid": agent_pid, "agent_instances": len(processes), "listeners": listeners, "tun": tun,
            "capabilities": int(proc["CapEff"].strip(), 16),
            "identity": json.loads(STATE.read_text())["node_id"] if STATE.exists() else None,
            "dns_cache_present": (ROOT / "agent.json.dns.json").exists(),
            "resolv_digest": hashlib.sha256(Path("/etc/resolv.conf").read_bytes()).hexdigest()}


def main(action, data):
    if action == "controller-start":
        return start("controller", ["meshway-controller", "serve", "--bind", "0.0.0.0:8420", "--data-dir", str(ROOT / "controller")])
    if action == "controller-stop":
        return stop("controller")
    if action == "admin-token":
        return {"token": checked(["meshway-controller", "admin-token", "--data-dir", str(ROOT / "controller")]).strip()}
    if action == "prepare-enrollment":
        if STATE.exists() or data.get("name") not in ("Twin", "tWIN", "Solo") or not isinstance(data.get("token"), str):
            raise ValueError("invalid fresh enrollment")
        # This is Agent's local Identity disk format, NOT an HTTP DTO. Persist
        # before host SDK enrollment; no token or credential enters child argv.
        credential = secrets.token_hex(32)
        atomic(STATE, {"format_version": 1, "controller": CONTROLLER, "name": data["name"],
                       "enrollment_version": VERSION, "enrollment_token_hash": list(hashlib.sha256(data["token"].encode()).digest()),
                       "node_credential": credential, "node_id": None})
        return {"credential": credential, "version": VERSION}
    if action == "complete-enrollment":
        value = json.loads(STATE.read_text())
        if value["node_id"] is not None or re.fullmatch(r"[0-9a-f]{64}", data.get("node_id", "")) is None:
            raise ValueError("invalid enrollment completion")
        value["node_id"] = data["node_id"]
        atomic(STATE, value)
        # Validate the private format through the real CLI before any service.
        public = json.loads(checked(["meshway-agent", "status", "--state", str(STATE)]))
        if public["node_id"] != value["node_id"]:
            raise ValueError("CLI identity mismatch")
        return {"node_id": value["node_id"]}
    if action == "agent-start":
        command = ["meshway-agent", "run", "--state", str(STATE), "--dns-listen", "127.0.0.1:5353",
                   "--interval-seconds", "1", "--connection-lease-seconds", "10"]
        if data.get("dataplane") is True:
            command.extend(["--apply-dataplane", "--tun-name", "meshway0", "--tun-mtu", "1000"])
        return start("agent", command)
    if action == "agent-stop":
        return stop("agent")
    if action == "second-instance":
        if pid("agent") is None:
            raise ValueError("first Agent must be alive")
        # A different DNS port proves rejection is the shared state lock,
        # rather than failure to bind the first instance's DNS port.
        result = subprocess.run(["meshway-agent", "run", "--state", str(STATE), "--dns-listen", "127.0.0.1:5354"],
                                stdin=subprocess.DEVNULL, capture_output=True, timeout=3)
        return {"rejected": result.returncode != 0}
    if action == "observe":
        return observe()
    if action == "query":
        return query(data["name"], data["kind"], data["transport"])
    if action == "reverse":
        return {"name": ipaddress.ip_address(data["address"]).reverse_pointer}
    if action == "tools":
        help_text = checked(["meshway-agent", "run", "--help"])
        if "--dns-listen" not in help_text or "--apply-dataplane" not in help_text:
            raise ValueError("image requires managed-connections feature")
        checked(["dig", "-v"])
        return {"ready": True}
    if action == "dig":
        # Separate dnsutils smoke check. Full assertions use correlated wire
        # transactions, never dig's exit code or empty stdout as DNS semantics.
        output = checked(["dig", "@127.0.0.1", "-p", "5353", "+tries=1", "+time=2", "+noall", "+comments",
                          "+tcp" if data["transport"] == "tcp" else "+notcp", data["name"], data["kind"]])
        match = re.search(r"status: ([A-Z]+), id: (\d+)", output)
        if match is None:
            raise ValueError("missing dig transaction header")
        return {"rcode": match.group(1), "transaction_id": int(match.group(2))}
    raise ValueError("unknown action")


if __name__ == "__main__":
    try:
        print(json.dumps(main(sys.argv[1], json.load(sys.stdin))))
    except Exception:
        print("managed DNS helper failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
