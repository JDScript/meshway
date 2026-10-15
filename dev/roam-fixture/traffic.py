"""Inner Mesh traffic of the roaming row. Every record carries this container's
CLOCK_REALTIME in nanoseconds; the supervisor reads perturbation times from the
same clock (the containers share one kernel).

serve ADDRESS DIRECTORY STOP   UDP echo on 42424 and TCP echo on 42425 (B, C)
client PEER DIRECTORY STOP     100 pps UDP sequence, 10/s ping and one TCP
                               stream writing a line every 50 ms (A)
Both run until the file STOP exists.
"""
import json
import os
import select
import socket
import subprocess
import sys
import threading
import time

STOP = None
UDP_PORT, TCP_PORT = 42424, 42425


def now():
    return time.time_ns()


class Log:
    """Bounded line log, flushed twice a second."""

    def __init__(self, path):
        self.file = open(path, "w", buffering=1 << 16)
        self.lock = threading.Lock()
        self.flushed = time.monotonic()

    def write(self, *fields):
        with self.lock:
            self.file.write(" ".join(str(field) for field in fields) + "\n")
            if time.monotonic() - self.flushed >= 0.5:
                self.file.flush()
                self.flushed = time.monotonic()

    def close(self):
        with self.lock:
            self.file.flush()
            self.file.close()


def stopped():
    return os.path.exists(STOP)


def serve(address, directory):
    received = Log(os.path.join(directory, "udp-received.log"))
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp.bind((address, UDP_PORT))
    tcp = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    tcp.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    tcp.bind((address, TCP_PORT))
    tcp.listen(4)
    streams = []
    while not stopped():
        readable, _, _ = select.select([udp, tcp, *streams], [], [], 0.2)
        for ready in readable:
            if ready is udp:
                payload, peer = udp.recvfrom(2048)
                received.write(now(), payload.decode(errors="replace").split(" ", 1)[0])
                udp.sendto(payload, peer)
            elif ready is tcp:
                stream, _ = tcp.accept()
                stream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                streams.append(stream)
            else:
                try:
                    data = ready.recv(65536)
                except OSError:
                    data = b""
                if not data:
                    ready.close()
                    streams.remove(ready)
                    continue
                ready.sendall(data)
    received.close()


def client(peer, directory):
    events = Log(os.path.join(directory, "client.log"))
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp.connect((peer, UDP_PORT))
    udp.setblocking(False)

    def udp_loop():
        sequence = 0
        interval = 0.01
        due = time.monotonic()
        while not stopped():
            current = time.monotonic()
            if current >= due:
                try:
                    udp.send(f"{sequence} {now()}".encode())
                    events.write("usend", now(), sequence)
                except OSError as error:
                    events.write("uerror", now(), sequence, error.errno)
                sequence += 1
                due += interval
                if due < current - 1:
                    due = current
            timeout = max(0, min(due - time.monotonic(), 0.01))
            readable, _, _ = select.select([udp], [], [], timeout)
            while readable:
                try:
                    payload = udp.recv(2048)
                except BlockingIOError:
                    break
                except OSError as error:
                    events.write("uerror", now(), -1, error.errno)
                    break
                events.write("uecho", now(), payload.decode(errors="replace").split(" ", 1)[0])

    def tcp_loop():
        stream = socket.create_connection((peer, TCP_PORT), timeout=10)
        stream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        stream.settimeout(0.05)
        events.write("tconnect", now())
        line = 0
        buffered = b""
        due = time.monotonic()
        while not stopped():
            if time.monotonic() >= due:
                try:
                    stream.sendall(f"{line} {now()}\n".encode())
                except socket.timeout:
                    events.write("tblocked", now())
                except OSError as error:
                    events.write("terror", now(), error.errno)
                    return
                line += 1
                due += 0.05
            try:
                data = stream.recv(65536)
            except socket.timeout:
                continue
            except OSError as error:
                events.write("terror", now(), error.errno)
                return
            if not data:
                events.write("tclosed", now())
                return
            buffered += data
            while b"\n" in buffered:
                record, buffered = buffered.split(b"\n", 1)
                events.write("techo", now(), record.decode(errors="replace").split(" ", 1)[0])
        stream.close()

    def ping_loop():
        process = subprocess.Popen(["ping", "-n", "-D", "-i", "0.1", "-W", "1", peer],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        try:
            for line in process.stdout:
                if "bytes from" in line and "icmp_seq=" in line:
                    sequence = line.split("icmp_seq=", 1)[1].split()[0]
                    events.write("pecho", now(), sequence)
                if stopped():
                    break
        finally:
            process.terminate()
            process.wait(timeout=5)

    threads = [threading.Thread(target=target, daemon=True) for target in (udp_loop, tcp_loop, ping_loop)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    events.close()


def main():
    global STOP
    mode, address, directory, STOP = sys.argv[1:5]
    os.makedirs(directory, exist_ok=True)
    with open(os.path.join(directory, "started.json"), "w") as started:
        json.dump({"atNs": now(), "pid": os.getpid()}, started)
    {"serve": serve, "client": client}[mode](address, directory)


if __name__ == "__main__":
    main()
