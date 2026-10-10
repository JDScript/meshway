"""Source-side lease probe run inside A: one persistent TCP echo per round and,
every two seconds, a fresh UDP socket (a new flow, admitted locally), until the
TCP flow ends. The fresh-flow rate stays well inside the exit's projection budget.

Wall-clock times are recorded for comparison with the actual netmap leases;
the probe neither reads nor extends any authority.
"""
import json
import os
import socket
import sys
import time

target, ready, out, limit = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])


def echo(sock, payload):
    sock.sendall(payload)
    received = b""
    while len(received) < len(payload):
        chunk = sock.recv(len(payload) - len(received))
        if not chunk:
            raise OSError("closed")
        received += chunk
    if received != payload:
        raise OSError("echo mismatch")


def fresh_udp(payload):
    for _ in range(2):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.settimeout(2)
            try:
                udp.connect((target, 8080))
                local = udp.getsockname()
                udp.send(payload)
                while True:
                    if udp.recv(2048) == payload:
                        return "%s:%d" % local
            except OSError:
                continue
    return None


started = time.time()
while True:
    try:
        tcp = socket.create_connection((target, 8080), 3)
        break
    except OSError:
        if time.time() - started > 45:
            raise
        time.sleep(.1)
tcp.settimeout(2)
rounds, failure, sequence, fresh_at = [], None, 0, 0.0
deadline = time.time() + limit
while time.time() < deadline:
    sequence += 1
    payload = b"exit-lease-%d" % sequence
    at = time.time()
    try:
        echo(tcp, payload)
    except OSError as error:
        failure = {"at": at, "error": str(error) or type(error).__name__}
        break
    fresh = None
    if at - fresh_at >= 2:
        fresh_at = at
        fresh = fresh_udp(payload) or False
    rounds.append({"at": at, "tcp": True, "udp": fresh})
    if sequence == 1:
        open(ready, "w").close()
    time.sleep(.5)
tcp.close()
result = {"target": target, "startedAt": started, "rounds": rounds, "failure": failure}
temporary = out + ".tmp"
with open(temporary, "w") as file:
    json.dump(result, file, indent=2)
os.replace(temporary, out)
