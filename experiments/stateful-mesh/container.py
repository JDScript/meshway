#!/usr/bin/env python3
"""Kernel probes; node configuration exclusively uses normal controller APIs."""
import importlib.util
import json
from pathlib import Path
import socket
import sys
import time
import hashlib
import socketserver
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
    if action == "netem":
        before = current.canaries.run(["tc","-s","-j","qdisc","show","dev","eth0"])
        args = ["tc", "qdisc", "replace", "dev", "eth0", "root", "netem", "delay", "20ms", "10ms", "loss", "2%", "reorder", "25%", "50%"] if data["enabled"] else ["tc", "qdisc", "del", "dev", "eth0", "root"]
        result = current.canaries.run(args)
        after = current.canaries.run(["tc","-s","-j","qdisc","show","dev","eth0"])
        return {"configured": result.returncode == 0, "error": result.stderr[:512],
                "before": json.loads(before.stdout), "after": json.loads(after.stdout)}
    if action == "bulk-start":
        return current.canaries.base.start("bulk-server", ["python3", __file__, "bulk-server", data["address"]])
    if action == "bulk-probe":
        body = bytes(range(256)) * 4096
        with socket.create_connection((data["address"],43002),timeout=40) as sock:
            sock.sendall(body)
            sock.shutdown(socket.SHUT_WR)
            reply = bytearray()
            while len(reply) < 128:
                chunk = sock.recv(128-len(reply))
                if not chunk:
                    break
                reply.extend(chunk)
        return {"sent_bytes": len(body), "delivered": bytes(reply) == hashlib.sha256(body).hexdigest().encode(), "half_close_delay_seconds": 6}
    if action == "udp-burst":
        received = set()
        count = 256
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
            sock.connect((data["address"],43001))
            sock.setblocking(False)
            for i in range(count):
                sock.send(i.to_bytes(4,"big"))
                time.sleep(.0005)
                while True:
                    try:
                        payload=sock.recv(1200)
                        if len(payload)==4:
                            received.add(int.from_bytes(payload,"big"))
                    except BlockingIOError:
                        break
            sock.settimeout(.2)
            until=time.monotonic()+3
            while time.monotonic()<until:
                try:
                    payload=sock.recv(1200)
                    if len(payload)==4:
                        received.add(int.from_bytes(payload,"big"))
                except socket.timeout:
                    continue
        return {"sent":count,"received":len(received.intersection(range(count)))}
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
        elif sys.argv[1] == "bulk-server":
            class Handler(socketserver.BaseRequestHandler):
                def handle(self):
                    body=bytearray()
                    self.request.settimeout(40)
                    while len(body)<=4*1024*1024:
                        chunk=self.request.recv(65536)
                        if not chunk:
                            time.sleep(6)
                            self.request.sendall(hashlib.sha256(body).hexdigest().encode())
                            return
                        body.extend(chunk)
            class Server(socketserver.ThreadingTCPServer):
                allow_reuse_address=True
                daemon_threads=True
            with Server((sys.argv[2],43002),Handler) as server:
                server.serve_forever()
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
