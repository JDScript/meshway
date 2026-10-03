#!/usr/bin/env python3
"""Kernel probes; node configuration exclusively uses normal controller APIs."""
import importlib.util
import json
from pathlib import Path
import socket
import sys
import time
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("current_mesh", "/private/current.py")
current = importlib.util.module_from_spec(spec)
spec.loader.exec_module(current)


def udp_server(address):
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind((address, 43001))
        while True:
            data, peer = sock.recvfrom(1200)
            if (Path("/private") / "udp-hold").exists():
                (Path("/private") / "udp-observed").write_text("received")
                until = time.monotonic() + 10
                while (Path("/private") / "udp-hold").exists() and time.monotonic() < until:
                    time.sleep(.02)
            sock.sendto(data, peer)


def main(action, data):
    if action == "udp-hold":
        (Path("/private") / "udp-observed").unlink(missing_ok=True)
        (Path("/private") / "udp-hold").write_text("hold")
        return {"saved": True}
    if action == "udp-observed":
        return {"received": (Path("/private") / "udp-observed").exists()}
    if action == "udp-release":
        (Path("/private") / "udp-hold").unlink(missing_ok=True)
        return {"saved": True}
    if action == "udp-pending-start":
        (Path("/private") / "udp-result.json").unlink(missing_ok=True)
        return current.canaries.base.start("udp-pending", ["python3", __file__, "udp-pending", data["address"]])
    if action == "udp-result":
        path = Path("/private/udp-result.json")
        return json.loads(path.read_text()) if path.exists() else {"done": False}
    if action == "udp-start":
        return current.canaries.base.start("mesh-udp", ["python3", __file__, "udp-server", data["address"]])
    if action == "udp-probe":
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(2)
            try:
                sock.connect((data["address"], 43001))
                body = data["body"].encode()
                sock.send(body)
                return {"delivered": sock.recv(1200) == body}
            except OSError:
                return {"delivered": False}
    return current.main(action, data)


if __name__ == "__main__":
    try:
        if sys.argv[1] == "udp-server":
            udp_server(sys.argv[2])
        elif sys.argv[1] == "udp-pending":
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
                sock.settimeout(7)
                sock.connect((sys.argv[2],43001))
                sock.send(b"held-original-flow")
                try:
                    delivered = sock.recv(1200) == b"held-original-flow"
                except OSError:
                    delivered = False
                Path("/private/udp-result.json").write_text(json.dumps({"done": True, "delivered": delivered}))
        elif sys.argv[1] == "proxy":
            current.proxy()
        elif sys.argv[1] == "http-server":
            current.canaries.serve()
        else:
            print(json.dumps(main(sys.argv[1], json.load(sys.stdin))))
    except Exception as error:
        (Path("/private") / "fixture-error.json").write_text(json.dumps({"action": sys.argv[1], "exception": type(error).__name__, "errno": getattr(error, "errno", None)}))
        print("stateful fixture helper failed", file=sys.stderr)
        sys.exit(1)
