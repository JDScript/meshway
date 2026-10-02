#!/usr/bin/env python3
"""Short RFC 1928/1929 functional tests against pinned Xray (no benchmarks).

Every Docker operation, including inspection/pull/start/cleanup, holds the shared
performance lock imported from this worktree. Native endpoints bind only random
loopback ports; containers have unique names/labels and no general proxy route.
"""

from contextlib import contextmanager
import io
import json
import os
from pathlib import Path
import selectors
import socket
import socketserver
import subprocess
import sys
import tarfile
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "scripts"))
from performance_lock import performance_lock  # noqa: E402

XRAY = "ghcr.io/xtls/xray-core@sha256:592ec4d11f656db95598d01e76dbcc6e002d67360b96a5436500a938230f52c7"
GREETING = b"meshway-socks5-acceptor-first\0\xff"
PAYLOAD = bytes(range(256)) * 256


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, timeout=30, **kwargs)


def docker(*args):
    return run("docker", *args).stdout.strip()


def wait_port(port):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return
        except OSError:
            time.sleep(0.05)
    raise RuntimeError("container listener did not become ready")


def read_exact(sock, count):
    result = bytearray()
    while len(result) < count:
        data = sock.recv(count - len(result))
        if not data:
            raise RuntimeError("truncated interop payload")
        result.extend(data)
    return bytes(result)


class Echo(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(10)
        self.request.sendall(GREETING)
        while data := self.request.recv(8192):
            self.request.sendall(data)


class EchoServer(socketserver.ThreadingTCPServer):
    daemon_threads = True


@contextmanager
def xray(config, label, network):
    name = f"meshway-socks5-{uuid.uuid4().hex[:12]}"
    # The pinned image runs as UID 65532. Preserve private permissions AND its
    # ownership via an archive copied as bytes (no host bind mounts/config files).
    contents = json.dumps(config).encode()
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode="w") as tar:
        entry = tarfile.TarInfo("config.json")
        entry.size, entry.mode, entry.uid, entry.gid = len(contents), 0o600, 65532, 65532
        tar.addfile(entry, io.BytesIO(contents))
    try:
        docker("create", "--name", name, "--label", label,
               "--network", network,
               "--publish", "127.0.0.1::1080", XRAY,
               "run", "-config", "/config.json")
        subprocess.run(["docker", "cp", "--archive", "-", f"{name}:/"],
                       input=archive.getvalue(), capture_output=True, timeout=30, check=True)
        docker("start", name)
        # This fixture only publishes the explicitly requested loopback port.
        info = json.loads(docker("inspect", name))[0]
        if not info["State"]["Running"]:
            raise RuntimeError("Xray fixture exited; private configuration details suppressed")
        bindings = info["NetworkSettings"]["Ports"]["1080/tcp"]
        if len(bindings) != 1 or bindings[0]["HostIp"] != "127.0.0.1":
            raise RuntimeError("unexpected non-loopback Docker binding")
        port = int(bindings[0]["HostPort"])
        wait_port(port)
        yield port
    finally:
        subprocess.run(["docker", "rm", "--force", name], capture_output=True, timeout=20)


@contextmanager
def our_server(probe, mode, env):
    child = subprocess.Popen([str(probe), "server", "mesh.example", "8443", mode],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ)
            if not selector.select(8):
                raise RuntimeError("our SOCKS5 fixture did not become ready")
            line = child.stdout.readline().strip()
        if not line.startswith("PORT "):
            raise RuntimeError("our SOCKS5 fixture did not report a listener")
        yield int(line.removeprefix("PORT "))
    finally:
        child.terminate()
        try:
            child.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.communicate(timeout=5)


def server_config(mode, env, port):
    settings = {"auth": "noauth", "udp": False}
    if mode == "password":
        settings.update(auth="password", accounts=[{
            "user": env["MESHWAY_SOCKS5_TEST_USERNAME"],
            "pass": env["MESHWAY_SOCKS5_TEST_PASSWORD"],
        }])
    return {
        "log": {"access": "none", "loglevel": "none"},
        "inbounds": [{"listen": "0.0.0.0", "port": 1080, "protocol": "socks", "settings": settings}],
        "outbounds": [
            {"tag": "deny", "protocol": "blackhole", "settings": {}},
            {"tag": "echo-only", "protocol": "freedom", "settings": {}},
        ],
        "routing": {"domainStrategy": "AsIs", "rules": [
            {"type": "field", "domain": ["full:host.docker.internal"], "port": str(port),
             "network": "tcp", "outboundTag": "echo-only"},
            {"type": "field", "network": "tcp,udp", "outboundTag": "deny"},
        ]},
    }


def client_config(mode, env, server_port, target="mesh.example"):
    proxy = {"address": "host.docker.internal", "port": server_port}
    if mode == "password":
        proxy["users"] = [{"user": env["MESHWAY_SOCKS5_TEST_USERNAME"],
                           "pass": env["MESHWAY_SOCKS5_TEST_PASSWORD"]}]
    return {
        "log": {"access": "none", "loglevel": "none"},
        "inbounds": [{"listen": "0.0.0.0", "port": 1080, "protocol": "dokodemo-door",
                      "settings": {"address": target, "port": 8443, "network": "tcp"}}],
        "outbounds": [{"protocol": "socks", "settings": {"servers": [proxy]}}],
    }


def upstream_client_probe(port):
    with socket.create_connection(("127.0.0.1", port), timeout=8) as sock:
        sock.settimeout(8)
        # Send before receiving: Xray's dokodemo dispatcher may wait for first
        # inbound data. The embedded acceptor itself always writes its greeting first.
        sock.sendall(PAYLOAD)
        if read_exact(sock, len(GREETING)) != GREETING:
            raise RuntimeError("incorrect upstream-client greeting")
        if read_exact(sock, len(PAYLOAD)) != PAYLOAD:
            raise RuntimeError("incorrect upstream-client binary echo")


@contextmanager
def network_fixture(label):
    name = f"meshway-socks5-{uuid.uuid4().hex[:12]}"
    try:
        docker("network", "create", "--label", label, name)
        yield name
    finally:
        subprocess.run(["docker", "network", "rm", name], capture_output=True, timeout=20)


def version(label):
    name = f"meshway-socks5-version-{uuid.uuid4().hex[:12]}"
    try:
        return docker("run", "--name", name, "--label", label, "--network", "none", XRAY, "version")
    finally:
        subprocess.run(["docker", "rm", "--force", name], capture_output=True, timeout=20)


def main():
    if not os.environ.get("MESHWAY_PERF_LOCK"):
        raise RuntimeError("set MESHWAY_PERF_LOCK to the shared Meshway performance lock")
    # Native compilation doesn't consume the shared Docker VM.
    subprocess.run(["cargo", "build", "--locked", "-p", "meshway-transport-socks5", "--example", "interop_probe"], cwd=ROOT, check=True)
    metadata = json.loads(run("cargo", "metadata", "--no-deps", "--format-version=1", cwd=ROOT).stdout)
    probe = Path(metadata["target_directory"]) / "debug" / "examples" / "interop_probe"
    env = os.environ | {"MESHWAY_SOCKS5_TEST_USERNAME": uuid.uuid4().hex,
                        "MESHWAY_SOCKS5_TEST_PASSWORD": uuid.uuid4().hex}
    label = f"io.meshway.socks5-interop={uuid.uuid4().hex}"
    with performance_lock(), network_fixture(label) as network:
        docker("image", "inspect", XRAY)
        print(version(label), flush=True)
        print(f"Pinned source: {XRAY}", flush=True)
        with EchoServer(("127.0.0.1", 0), Echo) as echo:
            worker = threading.Thread(target=echo.serve_forever, daemon=True)
            worker.start()
            try:
                for mode in ("noauth", "password"):
                    with xray(server_config(mode, env, echo.server_address[1]), label, network) as port:
                        args = (str(probe), "client", f"127.0.0.1:{port}", "host.docker.internal", str(echo.server_address[1]), mode)
                        result = run(*args, env=env)
                        print(f"{mode}: {result.stdout.strip()}", flush=True)
                        # Same proxy must not reach any target outside the allow rule.
                        denied = subprocess.run([*args[:4], "1", mode], env=env, capture_output=True, timeout=12)
                        if denied.returncode == 0:
                            raise RuntimeError("Xray's fixed echo route was bypassed")
                        if mode == "password":
                            wrong = env | {"MESHWAY_SOCKS5_TEST_PASSWORD": "wrong-fixture-password"}
                            denied = subprocess.run(args, env=wrong, capture_output=True, timeout=12)
                            if denied.returncode == 0 or b"AuthenticationFailed" not in denied.stderr:
                                raise RuntimeError("Xray did not reject incorrect credentials")
                            print("password: upstream rejects incorrect credentials", flush=True)
                        print(f"{mode}: upstream denies off-target payload", flush=True)
                    with our_server(probe, mode, env) as server_port:
                        with xray(client_config(mode, env, server_port), label, network) as port:
                            upstream_client_probe(port)
                            print(f"{mode}: PASS Xray client -> our fixed-target server, 65536-byte binary echo", flush=True)
                        with xray(client_config(mode, env, server_port, target="other.example"), label, network) as port:
                            try:
                                upstream_client_probe(port)
                            except (OSError, RuntimeError):
                                print(f"{mode}: PASS our server rejects Xray off-target CONNECT", flush=True)
                            else:
                                raise RuntimeError("our fixed target was bypassed")
            finally:
                echo.shutdown()
                worker.join(timeout=5)
    print("PASS all SOCKS5 Xray interoperability checks (functional only)")


if __name__ == "__main__":
    main()
