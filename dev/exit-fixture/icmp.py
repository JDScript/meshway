"""In-container tools of the ICMP error phase; the daemons own admission.

listen OUT SECONDS      A: every ICMPv4 error A's stack receives (raw socket),
                        with its sender, type, code, next-hop MTU and quote
udp TARGET PORT OUT SECONDS
                        A: one live UDP flow (fixed local port) to TARGET
tcp TARGET PORT BYTES   A: TCP echo of BYTES through a narrower downstream link
inject SOURCE DESTINATION COUNT QUOTED_SOURCE QUOTED_SPORT QUOTED_DESTINATION QUOTED_DPORT
                        router: COUNT time-exceeded errors quoting one UDP packet
"""
import json
import os
import socket
import struct
import sys
import time


def checksum(data):
    if len(data) % 2:
        data += b"\0"
    total = sum(struct.unpack("!%dH" % (len(data) // 2), data))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return ~total & 0xFFFF


def ipv4(source, destination, protocol, payload, ttl=64):
    header = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(payload), 0, 0x4000, ttl, protocol, 0,
                         socket.inet_aton(source), socket.inet_aton(destination))
    header = header[:10] + struct.pack("!H", checksum(header)) + header[12:]
    return header + payload


def save(path, value):
    with open(path + ".tmp", "w") as stream:
        json.dump(value, stream)
    os.replace(path + ".tmp", path)


def listen(out, seconds):
    sock = socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_ICMP)
    sock.settimeout(0.2)
    received = []
    deadline = time.monotonic() + seconds
    save(out, {"listening": True, "errors": received})
    while time.monotonic() < deadline:
        try:
            packet, _ = sock.recvfrom(65535)
        except socket.timeout:
            continue
        header = (packet[0] & 15) * 4
        kind, code = packet[header], packet[header + 1]
        if kind not in (3, 11):
            continue
        quoted = packet[header + 8:]
        inner = (quoted[0] & 15) * 4 if quoted else 0
        entry = {"at": time.time(), "source": socket.inet_ntoa(packet[12:16]), "type": kind, "code": code,
                 "mtu": struct.unpack("!H", packet[header + 6:header + 8])[0]}
        if len(quoted) >= inner + 8:
            entry["quoted"] = {"source": socket.inet_ntoa(quoted[12:16]), "destination": socket.inet_ntoa(quoted[16:20]),
                               "protocol": quoted[9], "head": quoted[inner:inner + 8].hex()}
        received.append(entry)
        if len(received) > 4096:
            break
        save(out, {"listening": True, "errors": received})
    save(out, {"listening": False, "errors": received})


def udp(target, port, out, seconds):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.connect((target, port))
    sock.settimeout(0.3)
    local = sock.getsockname()
    sent = answered = 0
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        sock.send(b"icmp-related-flow")
        sent += 1
        try:
            answered += sock.recv(64) == b"icmp-related-flow"
        except OSError:
            pass
        save(out, {"local": "%s:%d" % local, "sent": sent, "answered": answered})
        time.sleep(0.3)


def tcp(target, port, size):
    started = time.monotonic()
    payload = bytes(range(256)) * (size // 256)
    sock = socket.create_connection((target, port), 5)
    sock.settimeout(20)
    sock.sendall(payload)
    received = b""
    while len(received) < len(payload):
        chunk = sock.recv(65536)
        if not chunk:
            break
        received += chunk
    print(json.dumps({"bytes": len(received), "echoed": received == payload,
                      "seconds": round(time.monotonic() - started, 3),
                      "mss": sock.getsockopt(socket.IPPROTO_TCP, socket.TCP_MAXSEG)}))
    if received != payload:
        sys.exit(1)


def inject(source, destination, count, quoted_source, quoted_sport, quoted_destination, quoted_dport):
    datagram = struct.pack("!HHHH", quoted_sport, quoted_dport, 8 + 17, 0) + b"icmp-related-flow"
    quoted = ipv4(quoted_source, quoted_destination, 17, datagram, ttl=1)[:28]
    error = struct.pack("!BBHI", 11, 0, 0, 0) + quoted
    error = error[:2] + struct.pack("!H", checksum(error)) + error[4:]
    sock = socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_RAW)
    for _ in range(count):
        sock.sendto(ipv4(source, destination, 1, error), (destination, 0))
    print(json.dumps({"sent": count}))


if __name__ == "__main__":
    action, arguments = sys.argv[1], sys.argv[2:]
    if action == "listen":
        listen(arguments[0], float(arguments[1]))
    elif action == "udp":
        udp(arguments[0], int(arguments[1]), arguments[2], float(arguments[3]))
    elif action == "tcp":
        tcp(arguments[0], int(arguments[1]), int(arguments[2]))
    elif action == "inject":
        inject(arguments[0], arguments[1], int(arguments[2]), arguments[3], int(arguments[4]),
               arguments[5], int(arguments[6]))
    else:
        raise SystemExit(__doc__)
