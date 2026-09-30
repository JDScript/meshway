#!/usr/bin/env python3
"""Small, stdlib-only in-container helpers for the isolated transport lab."""

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import socketserver
import subprocess
import sys
import time

HEALTH_PORT = 9000
GREETING = b"meshway laboratory node\n"


class HealthHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.sendall(GREETING)


class HealthServer(socketserver.TCPServer):
    allow_reuse_address = True


def probe(host, port, timeout):
    try:
        with socket.create_connection((host, port), timeout=timeout) as connection:
            data = b""
            while not data.endswith(b"\n"):
                chunk = connection.recv(128)
                if not chunk:
                    return False
                data += chunk
            return data == GREETING
    except OSError:
        return False


def listening(port):
    # Inspect LISTEN state; connecting would consume an iperf3 --one-off server.
    for name in ("tcp", "tcp6"):
        path = Path("/proc/net") / name
        if path.exists():
            for line in path.read_text().splitlines()[1:]:
                fields = line.split()
                if fields[3] == "0A" and int(fields[1].split(":")[1], 16) == port:
                    return True
    return False


def cgroup_info():
    cgroup = {}
    for name in (
        "cpu.max", "cpu.stat", "cpuset.cpus.effective", "memory.max",
        "cpu/cpu.cfs_quota_us", "cpu/cpu.cfs_period_us", "cpu/cpu.stat",
        "cpuacct/cpuacct.usage", "cpu,cpuacct/cpuacct.usage",
    ):
        path = Path("/sys/fs/cgroup") / name
        if path.exists():
            cgroup[name] = path.read_text().strip()
    return {"monotonic_ns": time.monotonic_ns(), "files": cgroup}


def info():
    packages = (["apk", "info", "-v"] if shutil.which("apk") else
                ["dpkg-query", "-W", "-f=${Package}=${Version}\n"])
    return {
        "architecture": platform.machine(),
        "kernel": platform.release(),
        "uname": list(platform.uname()),
        "logical_cpus_visible": os.cpu_count(),
        "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "cpuinfo": Path("/proc/cpuinfo").read_text(),
        "cgroup": cgroup_info()["files"],
        "iperf3_version": subprocess.check_output(
            ["iperf3", "--version"], text=True, timeout=5
        ).strip(),
        "packages": subprocess.check_output(
            packages, text=True, timeout=5
        ).splitlines(),
    }


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("serve")
    commands.add_parser("info")
    commands.add_parser("cgroup")
    listen_parser = commands.add_parser("listening")
    listen_parser.add_argument("port", type=int)
    probe_parser = commands.add_parser("probe")
    probe_parser.add_argument("host")
    probe_parser.add_argument("--port", type=int, default=HEALTH_PORT)
    probe_parser.add_argument("--timeout", type=float, default=0.7)
    args = parser.parse_args()
    if args.command == "serve":
        with HealthServer(("0.0.0.0", HEALTH_PORT), HealthHandler) as server:
            server.serve_forever()
    elif args.command == "info":
        print(json.dumps(info()))
    elif args.command == "cgroup":
        print(json.dumps(cgroup_info()))
    elif args.command == "listening":
        sys.exit(0 if listening(args.port) else 1)
    elif args.command == "probe":
        sys.exit(0 if probe(args.host, args.port, args.timeout) else 1)


if __name__ == "__main__":
    main()
