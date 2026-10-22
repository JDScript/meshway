"""The fault row's business flow: a 5 pps UDP sequence A→B over the Mesh
(through R), low enough that the daemons stay idle otherwise. Every record
carries this container's CLOCK_REALTIME in nanoseconds (the containers share
one kernel with the controller the supervisor reads its times from).

serve ADDRESS DIRECTORY STOP   UDP echo on 42424 (B)
client PEER DIRECTORY STOP     one datagram every 200 ms; `s <ns> <seq>` per
                               send, `e <ns> <seq>` per echo (A)
Both run until the file STOP exists.
"""
import os
import select
import socket
import sys
import time

PORT = 42424
INTERVAL = 0.2


def serve(address, directory, stop):
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp.bind((address, PORT))
    open(os.path.join(directory, "serving"), "w").close()
    while not os.path.exists(stop):
        readable, _, _ = select.select([udp], [], [], 0.2)
        if readable:
            payload, peer = udp.recvfrom(2048)
            udp.sendto(payload, peer)


def client(peer, directory, stop):
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp.connect((peer, PORT))
    udp.setblocking(False)
    with open(os.path.join(directory, "client.log"), "w", buffering=1) as log:
        sequence, due = 0, time.monotonic()
        while not os.path.exists(stop):
            now = time.monotonic()
            if now >= due:
                try:
                    udp.send(str(sequence).encode())
                    log.write(f"s {time.time_ns()} {sequence}\n")
                except OSError:
                    pass
                sequence += 1
                due = max(due + INTERVAL, now - 1)
            readable, _, _ = select.select([udp], [], [], max(0.0, due - time.monotonic()))
            while readable:
                try:
                    payload = udp.recv(2048)
                except OSError:
                    break
                if payload.isdigit():
                    log.write(f"e {time.time_ns()} {int(payload)}\n")
                readable, _, _ = select.select([udp], [], [], 0)


if __name__ == "__main__":
    mode, target, directory, stop = sys.argv[1:5]
    os.makedirs(directory, exist_ok=True)
    (serve if mode == "serve" else client)(target, directory, stop)
