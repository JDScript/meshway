#!/usr/bin/env python3
"""Run the exact ignored Rust test against one isolated, pinned upstream Xray."""

import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import uuid


IMAGE = "ghcr.io/xtls/xray-core@sha256:592ec4d11f656db95598d01e76dbcc6e002d67360b96a5436500a938230f52c7"
LABEL = "io.meshway.vless-interop"
TEST_COMMAND = [
    "cargo", "test", "-p", "meshway-transport-vless", "--test", "external_proxy",
    "--", "--ignored", "--exact", "external_vless_tcp_connect_binary_round_trip",
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--temp-dir", type=Path, default=None)
    parser.add_argument("--probe-bind-ip", default="127.0.0.1")
    args = parser.parse_args()
    workspace = Path(__file__).resolve().parents[3]
    secret = str(uuid.uuid4())
    token = uuid.uuid4().hex
    name = f"meshway-vless-interop-{token[:12]}"
    network = f"{name}-net"
    network_created = False
    container_attempted = False

    def command(argv, *, timeout=30, check=True, show=False, env=None):
        result = subprocess.run(
            argv, cwd=workspace, env=env, capture_output=True, text=True,
            timeout=timeout, check=False,
        )
        output = (result.stdout + result.stderr).replace(secret, "[REDACTED]")
        if show or (check and result.returncode):
            print(output, end="", flush=True)
        if check and result.returncode:
            raise RuntimeError(f"command failed ({result.returncode}): {' '.join(argv)}")
        return result

    command(["docker", "info", "--format", "{{.ServerVersion}} {{.OperatingSystem}}"], show=True)
    if command(["docker", "image", "inspect", IMAGE], check=False).returncode:
        command(["docker", "pull", IMAGE], timeout=180, show=True)
    print(f"upstream image: {IMAGE}", flush=True)

    with tempfile.TemporaryDirectory(prefix=f"{name}-", dir=args.temp_dir) as temporary:
        config_path = Path(temporary) / "xray.json"
        # Reserve a currently unused host port for the separately launched Rust
        # listener. Bind again immediately before the test; collisions fail closed.
        with socket.socket() as reservation:
            reservation.bind((args.probe_bind_ip, 0))
            endpoint_port = reservation.getsockname()[1]
        config = {
            "log": {"loglevel": "warning"},
            "inbounds": [{
                "tag": "meshway-vless",
                "listen": "0.0.0.0", "port": 18443, "protocol": "vless",
                "settings": {"clients": [{"id": secret}], "decryption": "none"},
                "streamSettings": {"network": "tcp", "security": "none"},
            }],
            "outbounds": [
                {"tag": "blocked", "protocol": "blackhole", "settings": {}},
                {"tag": "meshway-endpoint", "protocol": "freedom", "settings": {}},
            ],
            "routing": {"domainStrategy": "AsIs", "rules": [
                {
                    "type": "field", "inboundTag": ["meshway-vless"],
                    "network": "tcp", "domain": ["full:host.docker.internal"],
                    "port": str(endpoint_port), "outboundTag": "meshway-endpoint",
                },
                {
                    "type": "field", "inboundTag": ["meshway-vless"],
                    "outboundTag": "blocked",
                },
            ]},
        }
        with config_path.open("x") as file:
            os.chmod(config_path, 0o600)
            json.dump(config, file)
        try:
            command([
                "docker", "network", "create", "--label", f"{LABEL}={token}", network,
            ])
            network_created = True
            container_attempted = True
            command([
                "docker", "run", "-d", "--name", name,
                "--label", f"{LABEL}={token}", "--network", network,
                "--read-only", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges",
                "--publish", "127.0.0.1::18443",
                "--mount", f"type=bind,src={config_path},dst=/etc/meshway-vless-test.json,readonly",
                IMAGE, "run", "-config", "/etc/meshway-vless-test.json",
            ])
            command(["docker", "exec", name, "/usr/local/bin/xray", "version"], show=True)
            command([
                "docker", "exec", name, "/usr/local/bin/xray", "run", "-test",
                "-config", "/etc/meshway-vless-test.json",
            ], show=True)
            published = command(["docker", "port", name, "18443/tcp"]).stdout.strip()
            host, port = published.rsplit(":", 1)
            if host != "127.0.0.1":
                raise RuntimeError("probe requires an exclusively loopback-published proxy")
            deadline = time.monotonic() + 10
            while True:
                try:
                    with socket.create_connection((host, int(port)), timeout=0.5):
                        break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise RuntimeError("Xray did not become ready within 10 seconds")
                    time.sleep(0.1)
            environment = os.environ.copy()
            environment.update({
                "MESHWAY_VLESS_UUID": secret,
                "MESHWAY_VLESS_PROXY": published,
                "MESHWAY_VLESS_BIND": f"{args.probe_bind_ip}:{endpoint_port}",
                "MESHWAY_VLESS_TARGET_HOST": "host.docker.internal",
                "MESHWAY_VLESS_EXPECT_DEFERRED": "1",
            })
            print(f"probe: {published} -> host.docker.internal:{endpoint_port}", flush=True)
            print("test command: " + " ".join(TEST_COMMAND), flush=True)
            command(TEST_COMMAND, timeout=300, show=True, env=environment)
        except BaseException:
            if container_attempted:
                command(["docker", "logs", name], check=False, show=True)
            raise
        finally:
            # Inspect the unguessable ownership label before deleting anything.
            if container_attempted:
                owned = command([
                    "docker", "inspect", "--format",
                    '{{index .Config.Labels "' + LABEL + '"}}', name,
                ], check=False)
                if owned.returncode == 0 and owned.stdout.strip() == token:
                    command(["docker", "rm", "--force", name])
            if network_created:
                command(["docker", "network", "rm", network])
    print("cleanup: owned container, network, and private temporary configuration removed", flush=True)


if __name__ == "__main__":
    main()
