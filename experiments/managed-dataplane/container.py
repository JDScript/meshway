#!/usr/bin/env python3
"""Private fixture helper: no controller HTTP implementation and no static lab plan.

Installed via stdin into each container's sole private tmpfs. Never returns logs,
private config, child errors or credentials except explicit bootstrap/bearer reads.
"""

import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import ipaddress
import json
import os
from pathlib import Path
import re
import select
import socket
import socketserver
import subprocess
import sys

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("managed_base", "/opt/meshway-managed/container.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
ROOT = Path("/private")
SERVER_PORT = 43000
POOL = ipaddress.IPv4Network("100.96.0.0/11")
os.umask(0o077)


def mesh_ip(value):
    address = ipaddress.IPv4Address(value)
    if address not in POOL:
        raise ValueError("expected controller mesh IPv4")
    return str(address)


def run(command, timeout=5):
    return subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False)


def snmp():
    lines = Path("/proc/net/snmp").read_text().splitlines()
    for i, line in enumerate(lines):
        if line.startswith("Icmp:"):
            values = dict(zip(line.split()[1:], map(int, lines[i + 1].split()[1:])))
            return {key: values[key] for key in ("InEchos", "InEchoReps", "OutEchoReps")}
    raise RuntimeError("missing ICMP counters")


def dataplane_proof():
    """Allowlisted local proof; never relay the Agent's raw stdout/stderr."""
    path = ROOT / "agent.log"
    latest = None
    ready_events = 0
    if path.exists():
        if path.stat().st_size > 16 * 1024 * 1024:
            raise RuntimeError("fixture log exceeds bounded inspection size")
        for line in path.read_text().splitlines():
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if not isinstance(value, dict) or value.get("event") != "managed_dataplane":
                continue
            # An old/unknown proof format must invalidate the latest observation,
            # never leave an earlier valid generation as the readiness evidence.
            latest = None
            revision = value.get("revision")
            digest = value.get("digest")
            fields = ("node_id", "revision", "digest", "tun_name", "ipv4", "mtu", "ipv6", "tun_ready",
                      "allowed_paths", "route_sink_retained", "reason")
            if (type(revision) is not int or not 1 <= revision <= 0xffffffff
                    or not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None
                    or not all(key in value for key in fields)):
                continue
            if value.get("tun_ready") is True:
                ready_events += 1
            latest = {key: value[key] for key in fields}
    return latest, ready_events


def route_observation(address):
    address = mesh_ip(address)
    result = run(["ip", "-j", "-4", "route", "get", address])
    if result.returncode == 0:
        routes = json.loads(result.stdout)
        if len(routes) != 1:
            raise RuntimeError("ambiguous route lookup")
        route = routes[0]
        return {"destination": address, "kind": "tun" if route.get("dev") == "meshway0" else "underlay",
                "device": route.get("dev"), "source": route.get("prefsrc"), "guard": None}
    # A removed TUN must leave an explicit guard. Mere lack of ping replies is
    # insufficient: an ordinary eth/default route would silently leak packets.
    candidates = []
    for route in json.loads(base.checked(["ip", "-j", "-4", "route", "show", "table", "all"])):
        destination = route.get("dst", "default")
        try:
            network = ipaddress.IPv4Network("0.0.0.0/0" if destination == "default" else destination)
        except ValueError:
            continue
        if network.subnet_of(POOL) and ipaddress.IPv4Address(address) in network and route.get("type") in ("blackhole", "unreachable", "prohibit"):
            candidates.append((network.prefixlen, str(network), route["type"]))
    guard = max(candidates) if candidates else None
    return {"destination": address, "kind": "guard" if guard else "missing", "device": None,
            "source": None, "guard": {"prefix": guard[1], "type": guard[2]} if guard else None}


def observe(data):
    status = base.sockets()
    status["icmp"] = snmp()
    status["routes"] = [route_observation(address) for address in data.get("destinations", [])]
    links = json.loads(base.checked(["ip", "-j", "address", "show"]))
    status["tun"] = [{"name": link["ifname"], "index": link["ifindex"], "mtu": link["mtu"],
                      "up": "UP" in link["flags"],
                      "ipv4": [entry["local"] for entry in link["addr_info"] if entry["family"] == "inet"],
                      "ipv6": [entry["local"] for entry in link["addr_info"] if entry["family"] == "inet6"]}
                     for link in links if link["ifname"] == "meshway0"]
    status["uid"] = os.getuid()
    proc = dict(line.split(":", 1) for line in Path("/proc/self/status").read_text().splitlines() if ":" in line)
    status["capabilities"] = int(proc["CapEff"].strip(), 16)
    status["ping_group_range"] = list(map(int, Path("/proc/sys/net/ipv4/ping_group_range").read_text().split()))
    status["ipv6_default_disabled"] = Path("/proc/sys/net/ipv6/conf/default/disable_ipv6").read_text().strip() == "1"
    agent_pid = base.pid("agent")
    status["agent_pid"] = agent_pid
    status["server_running"] = base.pid("mesh-http") is not None
    status["dataplane_proof"], status["ready_events"] = dataplane_proof()
    # Public receipt only; never return raw cache, state, log or arbitrary JSON.
    cache_path = ROOT / "agent.json.config.json"
    status["saved_revision"] = None
    if cache_path.exists():
        status["saved_revision"] = json.loads(cache_path.read_text())["snapshot"]["payload"]["revision"]
    return status


def probe(data):
    address = mesh_ip(data["address"])
    result = run(["ping", "-4", "-n", "-c", "1", "-W", "1", "-w", "2", address], timeout=4)
    # No interface override: exercise the Agent-installed kernel route itself.
    connection = http.client.HTTPConnection(address, SERVER_PORT, timeout=1.5)
    http_ok = False
    body_matches = False
    try:
        connection.request("GET", "/meshway-acceptance", headers={"Connection": "close"})
        response = connection.getresponse()
        payload = response.read(4097)
        http_ok = response.status == 200
        body_matches = payload == data["body"].encode()
    except (OSError, http.client.HTTPException):
        pass
    finally:
        connection.close()
    return {"ping_exit": result.returncode, "http_ok": http_ok, "body_matches": body_matches,
            "route": route_observation(address)}


def serve():
    config = json.loads((ROOT / "http.json").read_text())
    payload = config["body"].encode()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200 if self.path == "/meshway-acceptance" else 404)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *_args):
            pass

    # Binding exactly the assigned address excludes underlay/localhost fallback.
    ThreadingHTTPServer((mesh_ip(config["address"]), SERVER_PORT), Handler).serve_forever()


def proxy():
    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            try:
                with socket.create_connection(("controller.mesh.test", 8420), timeout=2) as upstream:
                    sockets = [self.request, upstream]
                    while True:
                        readable, _, _ = select.select(sockets, [], [], 10)
                        if not readable:
                            return
                        for source in readable:
                            payload = source.recv(65536)
                            if not payload:
                                return
                            (upstream if source is self.request else self.request).sendall(payload)
            except OSError:
                pass

    class Server(socketserver.ThreadingTCPServer):
        allow_reuse_address = True
        daemon_threads = True

    # Fixed upstream only: no CONNECT, host selection or general packet relay.
    with Server(("0.0.0.0", 8420), Handler) as server:
        server.serve_forever()


def main(action, data):
    if action == "agent-start":
        if os.getuid() != 0:
            raise RuntimeError("fixture agent requires its isolated root-owned directory")
        # Bound/clean logs per process; only local kernel observations are exported.
        (ROOT / "agent.log").write_text("")
        return base.start("agent", ["meshway-agent", "run", "--state", str(ROOT / "agent.json"),
                                   "--apply-dataplane", "--tun-name", "meshway0", "--tun-mtu", "1000",
                                   "--connection-profiles", str(ROOT / "profiles.json"),
                                   "--interval-seconds", "2", "--connection-lease-seconds", "15"])
    if action == "controller-proxy-start":
        return base.start("proxy", ["python3", __file__, "proxy"])
    if action == "observe":
        return observe(data)
    if action == "probe":
        return probe(data)
    if action == "http-start":
        address = mesh_ip(data["address"])
        if not isinstance(data["body"], str) or not re.fullmatch(r"meshway-tun:[a-z0-9-]+:(NYC|SHA)\n", data["body"]):
            raise ValueError("invalid public canary body")
        (ROOT / "http.json").write_text(json.dumps({"address": address, "body": data["body"]}))
        return base.start("mesh-http", ["python3", __file__, "http-server"])
    if action == "self-http":
        address = mesh_ip(data["address"])
        connection = http.client.HTTPConnection(address, SERVER_PORT, timeout=1)
        try:
            connection.request("GET", "/meshway-acceptance")
            response = connection.getresponse()
            return {"ready": response.status == 200 and response.read(4097) == data["body"].encode()}
        except OSError:
            return {"ready": False}
        finally:
            connection.close()
    if action == "loopback-ping":
        return {"ok": run(["ping", "-4", "-n", "-c", "1", "-W", "1", "127.0.0.1"]).returncode == 0}
    if action == "public-identity":
        # Compare controller-published key with the Agent's safe public CLI.
        value = json.loads(base.checked(["meshway-agent", "connections", "status", "--state", str(ROOT / "agent.json")]))
        return {"node_id": value["node_id"], "public_key": value["public_key"]}
    return base.main(action, data)


if __name__ == "__main__":
    try:
        if sys.argv[1] == "http-server":
            serve()
        elif sys.argv[1] == "proxy":
            proxy()
        else:
            print(json.dumps(main(sys.argv[1], json.load(sys.stdin))))
    except Exception:
        print("managed dataplane helper failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
