#!/usr/bin/env python3
"""Pinned sing-box TCP interoperability, both roles and all supported ciphers."""

import argparse
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import uuid


WORKSPACE = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(WORKSPACE / "experiments/transport"))
from performance_lock import performance_lock

IMAGE = "ghcr.io/sagernet/sing-box@sha256:3c1ee82d450df9b336b6f63b1d272cf86e3835392d99596ef8c35f206a457999"
LABEL = "io.meshway.shadowsocks-interop"
CIPHERS = ("chacha20-ietf-poly1305", "aes-128-gcm", "aes-256-gcm")
TEST = ["cargo", "test", "--locked", "-p", "meshway-transport-shadowsocks", "--test", "external_proxy"]


def run_cipher(cipher, args):
    secret = secrets.token_urlsafe(32)
    token = uuid.uuid4().hex
    name = f"meshway-ss-interop-{token[:12]}"
    network = f"{name}-net"
    network_created = False
    container_attempted = False

    def command(argv, *, timeout=60, check=True, show=False, env=None):
        result = subprocess.run(argv, cwd=WORKSPACE, env=env, capture_output=True,
                                text=True, timeout=timeout, check=False)
        output = (result.stdout + result.stderr).replace(secret, "[REDACTED]")
        if show or (check and result.returncode):
            print(output, end="", flush=True)
        if check and result.returncode:
            raise RuntimeError(f"command failed ({result.returncode}): {' '.join(argv)}")
        return result

    with tempfile.TemporaryDirectory(prefix=f"{name}-", dir=args.temp_dir) as temporary:
        config_path = Path(temporary) / "sing-box.json"
        with socket.socket() as endpoint, socket.socket() as adapter:
            endpoint.bind((args.probe_bind_ip, 0))
            adapter.bind((args.probe_bind_ip, 0))
            endpoint_port = endpoint.getsockname()[1]
            adapter_port = adapter.getsockname()[1]
        config = {
            "log": {"level": "error"},
            "dns": {"servers": [{"type": "local", "tag": "local"}]},
            "inbounds": [
                {"type": "shadowsocks", "tag": "ss-in", "listen": "0.0.0.0",
                 "listen_port": 18443, "network": "tcp", "method": cipher, "password": secret},
                {"type": "socks", "tag": "socks-in", "listen": "0.0.0.0", "listen_port": 18444},
            ],
            "outbounds": [
                {"type": "direct", "tag": "endpoint"},
                {"type": "shadowsocks", "tag": "adapter", "server": "host.docker.internal",
                 "server_port": adapter_port, "method": cipher, "password": secret, "network": "tcp"},
            ],
            "route": {"default_domain_resolver": "local", "rules": [
                {"inbound": ["ss-in"], "network": "tcp", "domain": ["host.docker.internal"],
                 "port": [endpoint_port], "action": "route", "outbound": "endpoint"},
                {"inbound": ["socks-in"], "network": "tcp", "domain": ["meshway.internal"],
                 "port": [19001], "action": "route", "outbound": "adapter"},
                {"action": "reject"},
            ]},
        }
        with os.fdopen(os.open(config_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as file:
            json.dump(config, file)
        try:
            command(["docker", "network", "create", "--label", f"{LABEL}={token}", network])
            network_created = True
            container_attempted = True
            command([
                "docker", "run", "-d", "--name", name, "--label", f"{LABEL}={token}",
                "--network", network, "--read-only", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges",
                "--publish", "127.0.0.1::18443", "--publish", "127.0.0.1::18444",
                "--mount", f"type=bind,src={config_path},dst=/etc/meshway-ss-test.json,readonly",
                IMAGE, "run", "-c", "/etc/meshway-ss-test.json",
            ])
            command(["docker", "exec", name, "sing-box", "version"], show=True)
            command(["docker", "exec", name, "sing-box", "check", "-c", "/etc/meshway-ss-test.json"], show=True)
            published = []
            for container_port in (18443, 18444):
                address = command(["docker", "port", name, f"{container_port}/tcp"]).stdout.strip()
                host, port = address.rsplit(":", 1)
                if host != "127.0.0.1":
                    raise RuntimeError("proxy must be published exclusively on loopback")
                deadline = time.monotonic() + 10
                while True:
                    try:
                        with socket.create_connection((host, int(port)), timeout=0.5):
                            break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise RuntimeError("sing-box readiness deadline")
                        time.sleep(0.1)
                published.append(address)
            environment = os.environ.copy()
            environment.update({
                "MESHWAY_SS_PASSWORD": secret,
                "MESHWAY_SS_CIPHER": cipher,
                "MESHWAY_SS_PROXY": published[0],
                "MESHWAY_SS_SOCKS": published[1],
                "MESHWAY_SS_ENDPOINT_BIND": f"{args.probe_bind_ip}:{endpoint_port}",
                "MESHWAY_SS_ADAPTER_BIND": f"{args.probe_bind_ip}:{adapter_port}",
            })
            print(f"cipher: {cipher}; Meshway -> sing-box and sing-box -> Meshway", flush=True)
            command(TEST + ["--", "--ignored", "--test-threads=1"], timeout=90, show=True, env=environment)
        except BaseException:
            if container_attempted:
                command(["docker", "logs", name], check=False, show=True)
            raise
        finally:
            # Only clean our unguessably named and ownership-labelled resources.
            if container_attempted:
                owned = command(["docker", "inspect", "--format",
                                 '{{index .Config.Labels "' + LABEL + '"}}', name], check=False)
                if owned.returncode == 0 and owned.stdout.strip() == token:
                    command(["docker", "rm", "--force", name])
            if network_created:
                command(["docker", "network", "rm", network])
    print("cleanup: owned container/network/private configuration removed", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--temp-dir", type=Path, default=None)
    parser.add_argument("--probe-bind-ip", default="127.0.0.1")
    args = parser.parse_args()
    # Compile before acquiring the Docker lock. Functional tests hold the same
    # host lock as all transport performance jobs for their entire lifetime.
    subprocess.run(TEST + ["--no-run"], cwd=WORKSPACE, check=True, timeout=300)
    if not os.environ.get("MESHWAY_PERF_LOCK"):
        parser.error("set MESHWAY_PERF_LOCK to the shared host performance lock")
    with performance_lock():
        subprocess.run(["docker", "info", "--format", "{{.ServerVersion}} {{.OperatingSystem}}"], check=True)
        subprocess.run(["docker", "pull", IMAGE], check=True, timeout=240)
        print(f"pinned image: {IMAGE}", flush=True)
        for cipher in CIPHERS:
            run_cipher(cipher, args)


if __name__ == "__main__":
    main()
