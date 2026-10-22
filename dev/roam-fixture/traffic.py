"""Inner Mesh traffic of the roaming row. Every record carries this container's
CLOCK_REALTIME in nanoseconds; the supervisor reads perturbation times from the
same clock (the containers share one kernel).

serve ADDRESS DIRECTORY STOP   UDP echo on 42424 and TCP echo on 42425 (B, C)
client PEER DIRECTORY STOP     100 pps UDP sequence, 10/s ping and one TCP
                               stream writing a line every 50 ms (A)
stream PEER DIRECTORY STOP     100 pps UDP sequence only, one new 5-tuple (A)
flows PEER DIRECTORY STOP      a new UDP 5-tuple every 250 ms (its own source
                               port, see flow_port), one datagram each, its
                               echo awaited for 20 s (A)
probe PEER DIRECTORY STOP      one new TCP connection with one echoed line and
                               one new UDP flow with one echo, then exits (B)
Every mode but probe runs until the file STOP exists. serve also records
each new UDP source and every TCP peer it accepts (udp-sources.log,
tcp-peers.log).
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
FLOW_PORT_BASE, FLOW_PORTS = 20000, 4000


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
    sources = Log(os.path.join(directory, "udp-sources.log"))
    peers = Log(os.path.join(directory, "tcp-peers.log"))
    source = None
    streams = []
    while not stopped():
        readable, _, _ = select.select([udp, tcp, *streams], [], [], 0.2)
        for ready in readable:
            if ready is udp:
                payload, peer = udp.recvfrom(2048)
                received.write(now(), payload.decode(errors="replace").split(" ", 1)[0])
                if peer != source:
                    source = peer
                    sources.write(now(), f"{peer[0]}:{peer[1]}")
                udp.sendto(payload, peer)
            elif ready is tcp:
                stream, peer = tcp.accept()
                peers.write(now(), f"{peer[0]}:{peer[1]}")
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
    sources.close()
    peers.close()


def udp_stream(peer, events):
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

    return udp_loop


def stream(peer, directory):
    events = Log(os.path.join(directory, "client.log"))
    udp_stream(peer, events)()
    events.close()


def flow_port(identifier):
    """The source port of per-datagram flow `identifier`. A kernel-chosen
    ephemeral port is reused at random once its socket closes, and a reused
    5-tuple within A's flow idle timeout (60 s) is that earlier flow, kept on
    its original path: not a new flow. These ports are outside the ephemeral
    range and none repeats within FLOW_PORTS × 250 ms (1,000 s)."""
    return FLOW_PORT_BASE + identifier % FLOW_PORTS


def flows(peer, directory):
    """Each datagram is a new flow: the source picks its path at that packet."""
    events = Log(os.path.join(directory, "client.log"))
    open_flows = {}
    identifier = 0
    due = time.monotonic()
    while not stopped():
        current = time.monotonic()
        if current >= due:
            flow = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            flow.setblocking(False)
            port = flow_port(identifier)
            try:
                flow.bind(("0.0.0.0", port))
                flow.connect((peer, UDP_PORT))
                flow.send(f"f{identifier} {now()}".encode())
                events.write("fsend", now(), identifier, port)
                open_flows[flow] = (identifier, current + 20)
            except OSError as error:
                # Counted as a flow of its time that got no reply.
                events.write("ferror", now(), identifier, error.errno)
                flow.close()
            identifier += 1
            due += 0.25
        for flow, (_, expires) in list(open_flows.items()):
            if current >= expires:
                flow.close()
                del open_flows[flow]
        readable, _, _ = select.select(list(open_flows), [], [], max(0, min(due - time.monotonic(), 0.05)))
        for flow in readable:
            try:
                payload = flow.recv(2048)
            except OSError:
                continue
            events.write("fecho", now(), payload.decode(errors="replace").split(" ", 1)[0][1:])
            flow.close()
            del open_flows[flow]
    events.close()


def client(peer, directory):
    events = Log(os.path.join(directory, "client.log"))
    udp_loop = udp_stream(peer, events)

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


def probe(peer, directory):
    """New flows: one TCP connection echoing one line, one UDP datagram."""
    result = {"startedAtNs": now()}
    try:
        with socket.create_connection((peer, TCP_PORT), timeout=5) as stream:
            stream.settimeout(5)
            stream.sendall(b"probe\n")
            data = b""
            while b"\n" not in data:
                chunk = stream.recv(64)
                if not chunk:
                    break
                data += chunk
            result["tcp"] = data == b"probe\n"
    except OSError as error:
        result["tcp"] = False
        result["tcpError"] = error.errno
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp.settimeout(1)
    result["udp"] = False
    try:
        udp.connect((peer, UDP_PORT))
        for _ in range(3):
            udp.send(b"p0 probe")
            try:
                result["udp"] = udp.recv(2048).startswith(b"p0 ")
                break
            except socket.timeout:
                continue
    except OSError as error:
        result["udpError"] = error.errno
    finally:
        udp.close()
    result["endedAtNs"] = now()
    with open(os.path.join(directory, "result.json"), "w") as out:
        json.dump(result, out)


def main():
    global STOP
    mode, address, directory, STOP = sys.argv[1:5]
    os.makedirs(directory, exist_ok=True)
    with open(os.path.join(directory, "started.json"), "w") as started:
        json.dump({"atNs": now(), "pid": os.getpid()}, started)
    {"serve": serve, "client": client, "stream": stream, "flows": flows, "probe": probe}[mode](address, directory)


if __name__ == "__main__":
    main()
