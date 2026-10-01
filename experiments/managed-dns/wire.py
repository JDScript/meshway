"""Bounded DNS wire client/parser. This module never serves or caches DNS."""
import ipaddress
import secrets
import socket
import struct

TYPES = {"A": 1, "AAAA": 28, "PTR": 12, "ANY": 255, "AXFR": 252}
RCODES = {0: "NOERROR", 1: "FORMERR", 2: "SERVFAIL", 3: "NXDOMAIN", 4: "NOTIMP", 5: "REFUSED"}


def name_at(packet, offset):
    labels, visited, end = [], set(), None
    while True:
        if offset >= len(packet) or offset in visited:
            raise ValueError("invalid DNS name bounds/cycle")
        visited.add(offset)
        size = packet[offset]
        offset += 1
        if size & 0xc0 == 0xc0:
            if offset >= len(packet):
                raise ValueError("truncated DNS pointer")
            pointer = ((size & 0x3f) << 8) | packet[offset]
            if end is None:
                end = offset + 1
            offset = pointer
            continue
        if size & 0xc0 or size > 63 or offset + size > len(packet):
            raise ValueError("invalid DNS label")
        if not size:
            return ".".join(labels).lower(), end if end is not None else offset
        labels.append(packet[offset:offset + size].decode("ascii"))
        if sum(len(label) + 1 for label in labels) > 254:
            raise ValueError("oversized DNS name")
        offset += size


def request(name, kind, identifier):
    labels = name.rstrip(".").split(".")
    if not labels or any(not label or len(label.encode("ascii")) > 63 for label in labels):
        raise ValueError("invalid query name")
    encoded = b"".join(bytes([len(label)]) + label.encode("ascii") for label in labels) + b"\0"
    if len(encoded) > 255:
        raise ValueError("oversized query name")
    return struct.pack("!6H", identifier, 0x0100, 1, 0, 0, 0) + encoded + struct.pack("!2H", TYPES[kind], 1)


def parse(packet, name, kind, identifier):
    if len(packet) < 12 or len(packet) > 65535:
        raise ValueError("invalid DNS message size")
    transaction, flags, questions, answers, authority, additional = struct.unpack_from("!6H", packet)
    if transaction != identifier or not flags & 0x8000 or flags & 0x7800 or questions != 1:
        raise ValueError("response transaction/header mismatch")
    question, offset = name_at(packet, 12)
    if offset + 4 > len(packet):
        raise ValueError("truncated question")
    qtype, qclass = struct.unpack_from("!2H", packet, offset)
    offset += 4
    if question != name.rstrip(".").lower() or qtype != TYPES[kind] or qclass != 1:
        raise ValueError("response question mismatch")
    sections = []
    for count in (answers, authority, additional):
        records = []
        for _ in range(count):
            owner, offset = name_at(packet, offset)
            if offset + 10 > len(packet):
                raise ValueError("truncated record header")
            rtype, rclass, ttl, length = struct.unpack_from("!HHIH", packet, offset)
            offset += 10
            end = offset + length
            if end > len(packet):
                raise ValueError("truncated record data")
            if rtype == 1 and length == 4:
                value = str(ipaddress.IPv4Address(packet[offset:end]))
            elif rtype == 28 and length == 16:
                value = str(ipaddress.IPv6Address(packet[offset:end]))
            elif rtype == 12:
                value, consumed = name_at(packet, offset)
                if consumed != end:
                    raise ValueError("invalid PTR data bounds")
            else:
                value = None
            records.append({"name": owner, "type": rtype, "class": rclass, "ttl": ttl, "value": value})
            offset = end
        sections.append(records)
    if offset != len(packet):
        raise ValueError("trailing DNS data")
    return {"transaction_id": transaction, "rcode": RCODES.get(flags & 15, str(flags & 15)),
            "ra": bool(flags & 0x80), "aa": bool(flags & 0x400), "tc": bool(flags & 0x200),
            "question": question, "type": kind, "answers": sections[0],
            "authority": sections[1], "additional": sections[2]}


def receive_exact(sock, length):
    result = bytearray()
    while len(result) < length:
        chunk = sock.recv(length - len(result))
        if not chunk:
            raise ValueError("truncated TCP DNS response")
        result.extend(chunk)
    return bytes(result)


def query(name, kind, transport, port=5353):
    identifier = secrets.randbelow(65536)
    payload = request(name, kind, identifier)
    if transport not in ("udp", "tcp"):
        raise ValueError("invalid transport")
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM if transport == "udp" else socket.SOCK_STREAM) as sock:
        sock.settimeout(2)
        sock.connect(("127.0.0.1", port))
        if transport == "udp":
            sock.send(payload)
            response = sock.recv(65536)
        else:
            sock.sendall(struct.pack("!H", len(payload)) + payload)
            response = receive_exact(sock, struct.unpack("!H", receive_exact(sock, 2))[0])
    return {**parse(response, name, kind, identifier), "transport": transport}
