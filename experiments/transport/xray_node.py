#!/usr/bin/env python3
"""Private Xray supervision and bounded, stdlib-only fixture checks."""

import argparse
import json
import os
from pathlib import Path
import signal
import socket
import socketserver
import ssl
import subprocess
import sys
import time

ROOT = Path("/run/xray-lab")
HEALTH_PORT = 19090
GREETING = b"isolated xray laboratory\n"
MAX_LOG_BYTES = 65536
TLS_NAME = "lax.transport-lab.invalid"


def save(name, value):
    temporary = ROOT / f".{name}.tmp"
    temporary.write_text(json.dumps(value))
    temporary.chmod(0o600)
    temporary.replace(ROOT / name)


def process_identity(pid):
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return {"state": fields[0], "start_ticks": int(fields[19])}
    except (OSError, ValueError, IndexError):
        return None


def state():
    record_path = ROOT / "xray-pid.json"
    result = {"running": False, "pid_recorded": record_path.exists()}
    if record_path.exists():
        record = json.loads(record_path.read_text())
        current = process_identity(record["pid"])
        result.update(pid=record["pid"], running=(current is not None and current["state"] != "Z"
                      and current["start_ticks"] == record["start_ticks"]))
    if (ROOT / "xray-exit.json").exists():
        result["exit"] = json.loads((ROOT / "xray-exit.json").read_text())
    return result


def supervise():
    try:
        child = subprocess.Popen(["xray", "run", "-config", str(ROOT / "config.json")],
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except OSError:
        save("xray-exit.json", {"spawn_failed": True})
        return 1
    identity = process_identity(child.pid)
    if identity is None:
        save("xray-exit.json", {"exit_code": child.wait(timeout=5), "early_exit": True})
        return 1
    save("xray-pid.json", {"pid": child.pid, "start_ticks": identity["start_ticks"]})
    raw_bytes = written = 0
    # Payload is completely redacted, even if a future Xray version prints config
    # fragments or PEM material. Keep draining after the cap to avoid backpressure.
    with (ROOT / "xray.log").open("wb") as log, child.stdout:
        while chunk := child.stdout.read1(8192):
            raw_bytes += len(chunk)
            line = f"[redacted diagnostic: {len(chunk)} bytes]\n".encode()
            kept = line[:max(0, MAX_LOG_BYTES - written)]
            log.write(kept)
            log.flush()
            written += len(kept)
    code = child.wait(timeout=5)
    save("xray-exit.json", {"exit_code": code, "diagnostic_bytes": raw_bytes,
                           "redacted_log_bytes": written, "log_at_cap": written == MAX_LOG_BYTES})
    return code


def tcp_sockets():
    result = []
    for line in Path("/proc/net/tcp").read_text().splitlines()[1:]:
        fields = line.split()

        def endpoint(value):
            address, port = value.split(":")
            return {"address": socket.inet_ntoa(bytes.fromhex(address)[::-1]), "port": int(port, 16)}

        result.append({"local": endpoint(fields[1]), "remote": endpoint(fields[2]), "state": fields[3]})
    return result


def socket_stats():
    files = {}
    for name in ("snmp", "netstat", "sockstat"):
        files[name] = (Path("/proc/net") / name).read_text()
    return {"monotonic_ns": time.monotonic_ns(), "files": files, "tcp_ipv4": tcp_sockets()}


class HealthHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(25)
        try:
            self.request.sendall(GREETING)
            # A held connection lets the host record the exact two proxy legs.
            while self.request.recv(64):
                pass
        except OSError:
            pass


class HealthServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def connect_health(host, hold=False):
    try:
        with socket.create_connection((host, HEALTH_PORT), timeout=1) as connection:
            data = b""
            while len(data) < len(GREETING):
                chunk = connection.recv(len(GREETING) - len(data))
                if not chunk:
                    return False
                data += chunk
            if data != GREETING:
                return False
            if hold:
                save("held-health.json", {"connected": True})
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline and not (ROOT / "release-health").exists():
                    time.sleep(0.05)
            return True
    except OSError:
        return False
    finally:
        if hold:
            save("held-health.json", {"connected": False})


def generate_tls(address):
    commands = [
        ["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
         "-keyout", "ca.key", "-out", "ca.crt", "-days", "1", "-subj", "/CN=Isolated Xray lab CA",
         "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign"],
        ["openssl", "req", "-new", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
         "-keyout", "server.key", "-out", "server.csr", "-subj", f"/CN={TLS_NAME}"],
        ["openssl", "x509", "-req", "-in", "server.csr", "-CA", "ca.crt", "-CAkey", "ca.key",
         "-CAcreateserial", "-out", "server.crt", "-days", "1", "-extfile", "server.ext"],
        ["openssl", "verify", "-CAfile", "ca.crt", "-verify_hostname", TLS_NAME, "server.crt"],
    ]
    (ROOT / "server.ext").write_text(
        f"subjectAltName=DNS:{TLS_NAME},IP:{address}\n"
        "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n"
    )
    for command in commands:
        result = subprocess.run(command, cwd=ROOT, capture_output=True, timeout=10)
        if result.returncode:
            raise RuntimeError("Private TLS certificate generation/verification failed; diagnostics suppressed")
    for name in ("ca.key", "ca.crt", "ca.srl", "server.key", "server.crt", "server.csr", "server.ext"):
        (ROOT / name).chmod(0o600)


def tls_check(address):
    def connect(context, name):
        with socket.create_connection((address, 41001), timeout=2) as raw:
            with context.wrap_socket(raw, server_hostname=name) as connection:
                return {"version": connection.version(), "cipher": connection.cipher(),
                        "subject_alt_name": connection.getpeercert().get("subjectAltName")}

    verified = ssl.create_default_context(cafile=str(ROOT / "ca.crt"))
    accepted = connect(verified, TLS_NAME)
    rejected = {}
    for label, context, name in (
        ("wrong_san_rejected", verified, "wrong.transport-lab.invalid"),
        ("untrusted_ca_rejected", ssl.create_default_context(), TLS_NAME),
    ):
        try:
            connect(context, name)
            rejected[label] = False
        except ssl.SSLCertVerificationError:
            rejected[label] = True
    if not all(rejected.values()):
        raise RuntimeError("TLS certificate negative check failed")
    return {"verified_handshake": accepted, **rejected}


def fault_client():
    command = ["iperf3", "--client", "127.0.0.1", "--bind", "127.0.0.1", "--port", "15201",
               "--time", "1", "--json", "--connect-timeout", "1000"]
    try:
        execution = subprocess.run(command, capture_output=True, text=True, timeout=5)
    except subprocess.TimeoutExpired:
        return {"command": command, "blocked": True, "timeout_seconds": 5, "child_killed_and_reaped": True}
    try:
        raw = json.loads(execution.stdout)
    except ValueError:
        raw = {}
    # A relay accepting locally and immediately closing can SIGPIPE iperf3 before
    # it emits JSON. This is a connection failure, not a successful control session.
    blocked = execution.returncode == -signal.SIGPIPE or bool(raw.get("error"))
    return {"command": command, "blocked": blocked,
            "exit_code": execution.returncode, "stderr": execution.stderr[:4096], "raw_client": raw}


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("supervise", "state", "stop", "socket-stats", "health-serve",
                                           "health-probe", "health-hold", "generate-tls", "tls-check", "fault-client", "validate"))
    parser.add_argument("address", nargs="?", default="127.0.0.1")
    args = parser.parse_args()
    if args.command == "supervise":
        return supervise()
    if args.command == "validate":
        result = subprocess.run(["xray", "run", "-test", "-config", str(ROOT / "config.json")],
                                capture_output=True, timeout=10)
        # Never propagate Xray configuration diagnostics into Docker errors/results.
        return result.returncode
    if args.command in ("state", "stop"):
        current = state()
        if args.command == "stop" and current["running"]:
            try:
                os.kill(current["pid"], signal.SIGINT)
            except ProcessLookupError:
                pass
        print(json.dumps(current))
    elif args.command == "socket-stats":
        print(json.dumps(socket_stats()))
    elif args.command == "health-serve":
        with HealthServer((args.address, HEALTH_PORT), HealthHandler) as server:
            server.serve_forever()
    elif args.command in ("health-probe", "health-hold"):
        return 0 if connect_health(args.address, args.command == "health-hold") else 1
    elif args.command == "generate-tls":
        generate_tls(args.address)
    elif args.command == "tls-check":
        print(json.dumps(tls_check(args.address)))
    elif args.command == "fault-client":
        print(json.dumps(fault_client()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
