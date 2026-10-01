#!/usr/bin/env python3
"""Independent held-socket filter fixture, explicitly separate from netcheck results."""

import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import socket
import sys
import time

from observer import atomic_snapshot
from wire import MAX_REQUEST, binding_request, endpoint, mapped_response

SNAPSHOT = Path("/private/canary.json")
SENT = Path("/private/canary.sent")


def binding(listener, server):
    transaction = os.urandom(12)
    listener.sendto(binding_request(transaction), server)
    deadline = time.monotonic() + 0.7
    while time.monotonic() < deadline:
        try:
            payload, peer = listener.recvfrom(MAX_REQUEST + 1)
        except socket.timeout:
            continue
        if peer[:2] == server:
            try:
                return mapped_response(payload, transaction)
            except ValueError:
                continue
    raise ValueError("legitimate control response not received")


def receive_canary(listener, peer, nonce, marker, maximum_wait=10, window=0.7):
    maximum = time.monotonic() + maximum_wait
    receive_deadline = None
    received = False
    while True:
        if marker.exists() and receive_deadline is None:
            receive_deadline = time.monotonic() + window
        if receive_deadline is not None and time.monotonic() >= receive_deadline:
            break
        if receive_deadline is None and time.monotonic() >= maximum:
            raise ValueError("canary send coordination did not finish")
        try:
            payload, source = listener.recvfrom(MAX_REQUEST + 1)
        except socket.timeout:
            continue
        if source[:2] == peer and payload == nonce:
            received = True
    return received


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--client", required=True, type=ipaddress.IPv4Address)
    parser.add_argument("--server-a", required=True, type=ipaddress.IPv4Address)
    parser.add_argument("--server-b", required=True, type=ipaddress.IPv4Address)
    parser.add_argument("--nonce", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{32}", args.nonce):
        raise ValueError("invalid canary nonce")
    nonce = b"meshway-filter-canary:" + args.nonce.encode("ascii")
    os.umask(0o077)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as listener:
        listener.bind((str(args.client), 0))
        listener.settimeout(0.05)
        server = (str(args.server_a), 3478)
        first = binding(listener, server)
        state = {"ready": True, "complete": False, "bound_socket": endpoint(*listener.getsockname()),
                 "first_mapping": first, "second_mapping": None, "legitimate_responses": 1,
                 "nonce_received": False, "same_socket_open": True, "receive_window_ms": 700}
        atomic_snapshot(SNAPSHOT, state)
        state["nonce_received"] = receive_canary(listener, (str(args.server_b), 3479), nonce, SENT)
        state["second_mapping"] = binding(listener, server)
        state["legitimate_responses"] = 2
        state["complete"] = True
        # Snapshot is written before exiting the with block: same socket remained
        # live throughout both legitimate responses and the bounded nonce window.
        atomic_snapshot(SNAPSHOT, state)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("held-socket canary helper failed", file=sys.stderr)
        sys.exit(1)
