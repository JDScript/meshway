#!/usr/bin/env python3
"""Fixture-only STUN observer; no control-plane identity, HTTP or raw packet logs."""

import argparse
import ipaddress
import json
import os
from pathlib import Path
import socket
import sys
import time

from wire import MAX_REQUEST, binding_response, endpoint

MAX_EVENTS = 128
RATE_PER_SECOND = 20


def atomic_snapshot(path, value):
    temporary = path.with_suffix(".next")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        json.dump(value, stream, allow_nan=False, separators=(",", ":"))
    os.replace(temporary, path)


def serve(listener, subnet, allowed_source, snapshot, stop=None):
    """The runner supplies one exact router WAN IP inside its auto-IPAM subnet."""
    if allowed_source not in subnet:
        raise ValueError("allowed source must be inside fixture subnet")
    listener.settimeout(0.1)
    events = []
    sequence = 0
    window = time.monotonic()
    received_in_window = 0
    state = {"ready": True, "server": endpoint(*listener.getsockname()[:2]), "sequence": 0, "events": []}
    atomic_snapshot(snapshot, state)
    while stop is None or not stop.is_set():
        try:
            request, peer = listener.recvfrom(MAX_REQUEST + 1)
        except socket.timeout:
            continue
        now = time.monotonic()
        if now - window >= 1:
            window, received_in_window = now, 0
        # Count all traffic, including malformed/source-rejected packets, in the bound.
        received_in_window += 1
        if received_in_window > RATE_PER_SECOND:
            continue
        address = ipaddress.ip_address(peer[0])
        if address not in subnet or address != allowed_source:
            continue
        try:
            response = binding_response(request, peer)
        except ValueError:
            continue
        listener.sendto(response, peer)
        sequence += 1
        events.append({"sequence": sequence, "observed_peer": endpoint(peer[0], peer[1])})
        events = events[-MAX_EVENTS:]
        state.update(sequence=sequence, events=events)
        atomic_snapshot(snapshot, state)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bind", required=True, type=ipaddress.ip_address)
    parser.add_argument("--subnet", required=True, type=ipaddress.ip_network)
    parser.add_argument("--source", required=True, type=ipaddress.ip_address)
    parser.add_argument("--snapshot", type=Path, default=Path("/private/observer.json"))
    args = parser.parse_args()
    family = socket.AF_INET if args.bind.version == 4 else socket.AF_INET6
    os.umask(0o077)
    with socket.socket(family, socket.SOCK_DGRAM) as listener:
        listener.bind((str(args.bind), 3478))
        serve(listener, args.subnet, args.source, args.snapshot)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("fixture observer failed", file=sys.stderr)
        sys.exit(1)
