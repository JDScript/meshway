"""Bounded RFC 8489 Binding helper. The mapped address is always recvfrom's peer."""

import ipaddress
import struct
import zlib

COOKIE = 0x2112A442
HEADER = struct.Struct("!HHI12s")
MAX_REQUEST = 512


def binding_request(transaction):
    if not isinstance(transaction, bytes) or len(transaction) != 12:
        raise ValueError("transaction must be 12 bytes")
    return HEADER.pack(0x0001, 0, COOKIE, transaction)


def binding_response(request, peer):
    """Accept bounded Binding requests, including SOFTWARE/FINGERPRINT attributes."""
    if not 20 <= len(request) <= MAX_REQUEST:
        raise ValueError("invalid Binding packet size")
    kind, length, cookie, transaction = HEADER.unpack_from(request)
    if kind != 0x0001 or cookie != COOKIE or length % 4 or length != len(request) - 20:
        raise ValueError("invalid Binding header")
    for kind, value, offset in attributes(request):
        if kind == 0x8028:
            if len(value) != 4 or offset + 8 != len(request):
                raise ValueError("FINGERPRINT must be last and four bytes")
            if struct.unpack("!I", value)[0] != zlib.crc32(request[:offset]) ^ 0x5354554E:
                raise ValueError("invalid FINGERPRINT")
        elif kind == 0x8022:
            value.decode("utf-8")
        elif kind < 0x8000:
            # No credential handling or authenticated STUN service in this fixture.
            raise ValueError("unsupported comprehension-required attribute")
    address = ipaddress.ip_address(peer[0])
    port = peer[1]
    if type(port) is not int or not 1 <= port <= 65535:
        raise ValueError("invalid source port")
    mask = struct.pack("!I", COOKIE) + transaction
    encoded = bytes(a ^ b for a, b in zip(address.packed, mask))
    value = struct.pack("!BBH", 0, 1 if address.version == 4 else 2, port ^ (COOKIE >> 16)) + encoded
    attribute = struct.pack("!HH", 0x0020, len(value)) + value
    return HEADER.pack(0x0101, len(attribute), COOKIE, transaction) + attribute


def attributes(packet):
    offset = 20
    while offset < len(packet):
        if offset + 4 > len(packet):
            raise ValueError("truncated attribute header")
        kind, size = struct.unpack_from("!HH", packet, offset)
        end = offset + 4 + size
        padded = offset + 4 + ((size + 3) & ~3)
        if end > len(packet) or padded > len(packet):
            raise ValueError("truncated attribute value or padding")
        yield kind, packet[offset + 4:end], offset
        offset = padded


def mapped_response(packet, transaction):
    """Bounded decoder for the held-socket canary control group, not an Agent substitute."""
    if not 20 <= len(packet) <= MAX_REQUEST:
        raise ValueError("invalid response size")
    kind, length, cookie, received = HEADER.unpack_from(packet)
    if kind != 0x0101 or cookie != COOKIE or received != transaction or length != len(packet) - 20 or length % 4:
        raise ValueError("invalid response header or transaction")
    result = None
    for kind, value, _offset in attributes(packet):
        if kind != 0x0020:
            continue
        if result is not None or len(value) not in (8, 20):
            raise ValueError("invalid XOR address attribute")
        reserved, family, port = struct.unpack_from("!BBH", value)
        if reserved != 0 or (family, len(value)) not in ((1, 8), (2, 20)):
            raise ValueError("invalid XOR address family")
        mask = struct.pack("!I", COOKIE) + transaction
        address = ipaddress.ip_address(bytes(a ^ b for a, b in zip(value[4:], mask)))
        result = endpoint(str(address), port ^ (COOKIE >> 16))
    if result is None:
        raise ValueError("missing XOR mapped address")
    return result


def endpoint(address, port):
    ip = ipaddress.ip_address(address)
    return f"[{ip}]:{port}" if ip.version == 6 else f"{ip}:{port}"
