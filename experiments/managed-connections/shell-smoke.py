#!/usr/bin/env python3
"""Tiny, isolated, shared-lock image/shell test. Never mounts demo data."""

import argparse
import json
import os
from pathlib import Path
import platform
import secrets
import signal
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1] / "scripts"))
from performance_lock import DEFAULT_LOCK, performance_lock
LOCK_FD = None


def docker(*args, input=None):
    if LOCK_FD is None:
        raise RuntimeError("shared performance lock required")
    result = subprocess.run(["docker", *args], input=input, capture_output=True, text=True, timeout=45, check=False, pass_fds=(LOCK_FD,))
    if result.returncode:
        raise RuntimeError("scratch Docker command failed; output suppressed")
    return result.stdout.strip()


def native(value):
    aliases = {"arm64": "arm64", "aarch64": "arm64", "amd64": "amd64", "x86_64": "amd64", "x64": "amd64"}
    if value not in aliases:
        raise RuntimeError("unsupported architecture")
    return aliases[value]


def smoke(image):
    uid, gid = os.getuid(), os.getgid()
    if uid == 0:
        raise RuntimeError("non-root host user required")
    info = json.loads(docker("image", "inspect", image, "--format", "{{json .}}"))
    if info["Os"] != "linux" or not native(platform.machine()) == native(docker("info", "--format", "{{.Architecture}}")) == native(info["Architecture"]):
        raise RuntimeError("host, Docker engine and image must be native; emulation refused")
    pin = info["Id"]
    run = secrets.token_hex(8)
    prefix = "meshway-shell-" + run
    network = prefix + "-net"
    labels = {"io.meshway.shell-smoke.run": run, "io.meshway.shell-smoke.project": "managed-neighbors"}
    label_args = [part for key, value in labels.items() for part in ("--label", key + "=" + value)]
    created = []
    net_created = False
    try:
        net_created = True
        docker("network", "create", "--internal", *label_args, network)
        for role in ("lax", "nyc"):
            name = prefix + "-" + role
            created.append(name)
            docker("create", "--name", name, "--pull", "never", *label_args,
                   "--network", network, "--network-alias", role + ".mesh.test",
                   "--user", f"{uid}:{gid}", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--read-only",
                   "--sysctl", "net.ipv4.ping_group_range=0 2147483647", "--sysctl", "net.ipv4.ip_forward=0",
                   "--sysctl", "net.ipv6.conf.all.forwarding=0", "--pids-limit", "32", "--memory", "128m", "--log-driver", "none",
                   "--mount", f"type=bind,src={HERE / 'demo.py'},dst=/opt/meshway-demo.py,readonly",
                   "--entrypoint", "sleep", pin, "infinity")
            docker("start", name)
        nyc = prefix + "-nyc"
        result = json.loads(docker("exec", "--interactive", nyc, "python3", "/opt/meshway-demo.py", "--container-image-check", input=json.dumps({"shell": True})))
        if result.get("managed") is not True:
            raise RuntimeError("managed binary validation failed")
        # ICMP Echo datagram sockets, with ALL capabilities dropped. This is only
        # Docker underlay connectivity; there is no Meshway identity or TUN here.
        docker("exec", nyc, "ping", "-c", "1", "-W", "2", "127.0.0.1")
        docker("exec", nyc, "ping", "-c", "1", "-W", "2", "lax.mesh.test")
        evidence = {"result": "PASS", "image_id": pin, "uid": uid, "gid": gid,
                          "architecture": info["Architecture"], "capabilities": "none", "rootfs": "read-only",
                          "underlay_ping": ["127.0.0.1", "lax.mesh.test"], "ping_cli_available": result["ping_cli"],
                          "authenticated_neighbor_ping_tested": False}
    finally:
        errors = []
        for name in reversed(created):
            try:
                if not docker("container", "ls", "--all", "--filter", "name=^/" + name + "$", "--format", "{{.Names}}"):
                    continue
                value = json.loads(docker("container", "inspect", name, "--format", "{{json .}}"))
                if value["Image"] != pin or any(value["Config"]["Labels"].get(key) != expected for key, expected in labels.items()):
                    raise RuntimeError("scratch container ownership differs")
                docker("rm", "--force", name)
            except Exception:
                errors.append(name)
        if net_created:
            try:
                if docker("network", "ls", "--filter", "name=^" + network + "$", "--format", "{{.Name}}"):
                    value = json.loads(docker("network", "inspect", network, "--format", "{{json .}}"))
                    if any(value["Labels"].get(key) != expected for key, expected in labels.items()):
                        raise RuntimeError("scratch network ownership differs")
                    docker("network", "rm", network)
            except Exception:
                errors.append(network)
        if errors:
            print("scratch cleanup incomplete: " + ", ".join(errors), file=sys.stderr)
            raise RuntimeError("scratch cleanup incomplete")
    print(json.dumps({**evidence, "cleanup": "complete"}))


def main():
    global LOCK_FD
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True, help="prebuilt native managed image; retained after test")
    args = parser.parse_args()
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    with performance_lock() as descriptor:
        LOCK_FD = descriptor
        smoke(args.image)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("shell smoke interrupted; scratch cleanup attempted", file=sys.stderr)
        sys.exit(130)
    except Exception:
        print("shell smoke failed; private exception details suppressed", file=sys.stderr)
        sys.exit(1)
