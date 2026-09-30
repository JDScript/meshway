#!/usr/bin/env python3
"""Private, bounded-log agent supervisor. Never exports configs or agent logs."""

import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys

ROOT = Path("/run/meshway-lab")
MAX_LOG_BYTES = 1024 * 1024


def save(name, value):
    destination = ROOT / name
    temporary = ROOT / f".{name}.tmp"
    temporary.write_text(json.dumps(value))
    temporary.chmod(0o600)
    temporary.replace(destination)


def process_identity(pid):
    try:
        # The comm field is parenthesized and may itself contain whitespace.
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return {"state": fields[0], "start_ticks": int(fields[19])}
    except (OSError, ValueError, IndexError):
        return None


def agent_state():
    pid_file = ROOT / "agent-pid.json"
    if not pid_file.exists():
        return {"running": False, "pid_recorded": False}
    record = json.loads(pid_file.read_text())
    current = process_identity(record["pid"])
    running = (current is not None and current["state"] != "Z" and
               current["start_ticks"] == record["start_ticks"])
    result = {"running": running, "pid_recorded": True, "pid": record["pid"]}
    exit_path = ROOT / "agent-exit.json"
    if exit_path.exists():
        result["exit"] = json.loads(exit_path.read_text())
    return result


def supervise():
    command = ["meshway-agent", "transport-lab", "run", "--config", str(ROOT / "config.json"),
               "--status-file", str(ROOT / "status.json")]
    try:
        child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except OSError:
        save("agent-exit.json", {"spawn_failed": True})
        return 1
    identity = process_identity(child.pid)
    if identity is None:
        save("agent-exit.json", {"exit_code": child.wait(timeout=5), "early_exit": True})
        return 1
    save("agent-pid.json", {"pid": child.pid, "start_ticks": identity["start_ticks"]})
    written = 0
    truncated = False
    # Continue draining after the cap so agent logging cannot block its data path.
    with (ROOT / "agent.log").open("wb") as log, child.stdout:
        while True:
            chunk = child.stdout.read(8192)
            if not chunk:
                break
            kept = chunk[:max(0, MAX_LOG_BYTES - written)]
            log.write(kept)
            log.flush()
            written += len(kept)
            truncated = truncated or len(kept) < len(chunk)
    code = child.wait(timeout=5)
    save("agent-exit.json", {"exit_code": code, "log_bytes": written, "log_truncated": truncated})
    return code


def tcp_sockets():
    sockets = []
    for line in Path("/proc/net/tcp").read_text().splitlines()[1:]:
        fields = line.split()

        def endpoint(value):
            address, port = value.split(":")
            return {"address": socket.inet_ntoa(bytes.fromhex(address)[::-1]), "port": int(port, 16)}

        sockets.append({"local": endpoint(fields[1]), "remote": endpoint(fields[2]), "state": fields[3]})
    return sockets


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("supervise", "state", "stop", "tcp-sockets"))
    args = parser.parse_args()
    if args.command == "supervise":
        return supervise()
    if args.command == "tcp-sockets":
        print(json.dumps(tcp_sockets()))
        return 0
    state = agent_state()
    if args.command == "stop" and state["running"]:
        try:
            os.kill(state["pid"], signal.SIGINT)
        except ProcessLookupError:
            pass
    print(json.dumps(state))
    return 0


if __name__ == "__main__":
    sys.exit(main())
