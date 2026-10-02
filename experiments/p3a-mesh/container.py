#!/usr/bin/env python3
"""Reuses kernel-only canaries; v3 activation exclusively uses generated HTTP."""
import importlib.util
import json
import os
from pathlib import Path
import select
import socket
import socketserver
import sys
import time
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("mesh_canaries", "/private/canaries.py")
canaries = importlib.util.module_from_spec(spec)
spec.loader.exec_module(canaries)
ROOT = Path("/private")

def proxy():
    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            try:
                first = self.request.recv(65536)
                mode = (ROOT / "stall").read_text() if (ROOT / "stall").exists() else ""
                if mode == "all" or (mode == "execution" and b" /api/v1/agents/mesh-execution " in first):
                    time.sleep(30)
                    return
                with socket.create_connection(("controller.mesh.test", 8420), timeout=2) as upstream:
                    upstream.sendall(first)
                    sockets = [self.request, upstream]
                    while True:
                        readable, _, _ = select.select(sockets, [], [], 10)
                        if not readable:
                            return
                        for source in readable:
                            data = source.recv(65536)
                            if not data:
                                return
                            (upstream if source is self.request else self.request).sendall(data)
            except OSError:
                pass
    class Server(socketserver.ThreadingTCPServer):
        allow_reuse_address = True
        daemon_threads = True
    with Server(("0.0.0.0", 8420), Handler) as server:
        server.serve_forever()

def main(action, data):
    if action == "agent-start":
        (ROOT / "agent.log").write_text("")
        return canaries.base.start("agent", ["meshway-agent", "run", "--state", str(ROOT / "agent.json"), "--apply-dataplane", "--tun-name", "meshway0", "--tun-mtu", "1000", "--interval-seconds", "2", "--connection-lease-seconds", "15"])
    if action == "controller-proxy-start":
        return canaries.base.start("proxy", ["python3", __file__, "proxy"])
    if action == "stall":
        if data["mode"] not in ("all", "execution", "off"):
            raise ValueError("invalid stall mode")
        (ROOT / "stall").write_text(data["mode"])
        return {"saved": True}
    if action == "execution-proof":
        path = ROOT / "agent.json.mesh-execution.json"
        if not path.exists():
            return {"revision": None, "digest": None, "paths": [], "valid": False}
        snapshot = json.loads(path.read_text())["snapshot"]
        return {"revision": snapshot["payload"]["revision"], "digest": snapshot["digest"], "paths": snapshot["payload"]["paths"], "valid": snapshot["payload"]["valid"]}
    if action == "agent-errors":
        path = ROOT / "agent.log"
        # Agent non-JSON errors are already redacted; omit every serialized
        # receipt and all arbitrary state/material files from diagnostics.
        lines = path.read_text().splitlines() if path.exists() else []
        return {"errors": [line for line in lines if not line.startswith("{")][-20:]}
    return canaries.main(action, data)

if __name__ == "__main__":
    try:
        if sys.argv[1] == "proxy":
            proxy()
        elif sys.argv[1] == "http-server":
            canaries.serve()
        else:
            print(json.dumps(main(sys.argv[1], json.load(sys.stdin))))
    except Exception:
        print("p3a helper failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
