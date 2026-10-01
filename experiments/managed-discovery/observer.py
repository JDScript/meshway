#!/usr/bin/env python3
"""Private bounded STUN observer, accepting only the two fixture Agent IPs."""
import ipaddress
import json
import os
from pathlib import Path
import signal
import socket
import sys
import time

sys.dont_write_bytecode = True
from stun_wire import binding_response

STATE = Path("/private/observer.json")
MAX_INPUT = 1024
RATE = 20


def private_ip(value):
    address = ipaddress.IPv4Address(value)
    if not address.is_private or address.is_loopback or address.is_unspecified or address.is_multicast or address.is_link_local:
        raise ValueError("only private bridge IPv4 is allowed")
    return str(address)


class Ledger:
    def __init__(self, allowed, previous=None):
        self.allowed = frozenset(private_ip(value) for value in allowed)
        if len(self.allowed) != 2:
            raise ValueError("exactly two distinct source IPs required")
        self.value = previous or {"count": 0, "peers": []}
        if any(peer["ip"] not in self.allowed for peer in self.value["peers"]):
            raise ValueError("ledger source mismatch")
        self.window = -1
        self.packets = 0

    def respond(self, request, peer, monotonic=None, wall=None):
        window = int(time.monotonic() if monotonic is None else monotonic)
        if window != self.window:
            self.window, self.packets = window, 0
        self.packets += 1
        if self.packets > RATE or peer[0] not in self.allowed or len(request) > MAX_INPUT:
            return None
        try:
            response = binding_response(request, peer)
        except (ValueError, UnicodeError):
            return None
        entry = next((item for item in self.value["peers"] if (item["ip"], item["port"]) == peer[:2]), None)
        stamp = time.time() if wall is None else wall
        if entry is None:
            if len(self.value["peers"]) >= 256:
                return None
            entry = {"ip": peer[0], "port": peer[1], "count": 0, "first_at": stamp, "last_at": stamp}
            self.value["peers"].append(entry)
        self.value["count"] += 1
        entry["count"] += 1
        entry["last_at"] = stamp
        return response


def main(config):
    bind = private_ip(config["bind"])
    previous = json.loads(STATE.read_text()) if STATE.exists() else None
    ledger = Ledger(config["allowed"], previous)
    os.umask(0o077)
    running = True

    def stop(_signum, _frame):
        nonlocal running
        running = False

    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind((bind, 3478))
        sock.settimeout(0.2)
        while running:
            try:
                request, peer = sock.recvfrom(MAX_INPUT + 1)
            except socket.timeout:
                continue
            response = ledger.respond(request, peer)
            if response is None:
                continue
            sock.sendto(response, peer)
            temporary = STATE.with_suffix(".new")
            temporary.write_text(json.dumps(ledger.value))
            temporary.replace(STATE)


if __name__ == "__main__":
    main(json.loads(sys.argv[1]))
