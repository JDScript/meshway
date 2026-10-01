"""Bounded DNS wire handling for the isolated Linux container experiment."""

import asyncio
import ipaddress
import math
import secrets
import socket
import struct
import time
from dataclasses import dataclass

MAX_MESSAGE = 4096
DEADLINE = 2.0
MAX_CONCURRENT = 32


class DNSInvalid(ValueError):
    pass


def zone_name(value):
    value = value.removesuffix(".").lower()
    if not value or len(value) > 253:
        raise ValueError("zone must be an ASCII DNS name")
    for label in value.split("."):
        if (not 1 <= len(label) <= 63 or not label.isascii()
                or not all(c.isalnum() or c == "-" for c in label)
                or label[0] == "-" or label[-1] == "-"):
            raise ValueError("zone must use valid ASCII labels (encode IDNs as punycode)")
    return value


def wire_name(name):
    return b"".join(bytes([len(part)]) + part.encode("ascii")
                    for part in name.split(".")) + b"\0"


def read_name(data, offset):
    labels, visited, end, total = [], set(), None, 1
    while True:
        if offset >= len(data) or offset in visited:
            raise DNSInvalid("invalid DNS name offset or compression loop")
        visited.add(offset)
        length = data[offset]
        if length & 0xC0 == 0xC0:
            if offset + 1 >= len(data):
                raise DNSInvalid("short compression pointer")
            pointer = ((length & 63) << 8) | data[offset + 1]
            if pointer >= offset:
                raise DNSInvalid("compression pointer must point backwards")
            if end is None:
                end = offset + 2
            offset = pointer
            continue
        if length & 0xC0:
            raise DNSInvalid("reserved label encoding")
        offset += 1
        if not length:
            return tuple(labels), end if end is not None else offset
        if offset + length > len(data):
            raise DNSInvalid("short label")
        total += length + 1
        if total > 255:
            raise DNSInvalid("name too long")
        labels.append(data[offset:offset + length].lower())
        offset += length


@dataclass
class Message:
    ident: int
    flags: int
    question: tuple
    question_end: int
    # (TTL byte offset, TTL, record type, section)
    records: list
    udp_size: int


def parse(data):
    if not 12 <= len(data) <= MAX_MESSAGE:
        raise DNSInvalid("DNS message length outside bounds")
    ident, flags, qd, an, ns, ar = struct.unpack_from("!6H", data)
    if qd != 1:
        raise DNSInvalid("exactly one question required")
    name, pos = read_name(data, 12)
    if pos + 4 > len(data):
        raise DNSInvalid("short question")
    kind, cls = struct.unpack_from("!HH", data, pos)
    pos += 4
    question_end, records, udp_size = pos, [], 512
    for section, count in enumerate((an, ns, ar)):
        for _ in range(count):
            owner, pos = read_name(data, pos)
            if pos + 10 > len(data):
                raise DNSInvalid("short resource record")
            rtype, rclass, ttl, size = struct.unpack_from("!HHIH", data, pos)
            ttl_pos = pos + 4
            start, end = pos + 10, pos + 10 + size
            if end > len(data):
                raise DNSInvalid("resource record exceeds packet")
            # Validate name-bearing/common RDATA, not just its outer length.
            if rtype in (2, 5, 12):
                _, tail = read_name(data, start)
                if tail != end:
                    raise DNSInvalid("invalid name RDATA")
            elif rtype in (15, 33):
                prefix = 2 if rtype == 15 else 6
                if start + prefix >= end:
                    raise DNSInvalid("short name RDATA")
                _, tail = read_name(data, start + prefix)
                if tail != end:
                    raise DNSInvalid("invalid name RDATA")
            elif rtype == 6:
                _, tail = read_name(data, start)
                _, tail = read_name(data, tail)
                if tail + 20 != end:
                    raise DNSInvalid("invalid SOA RDATA")
            elif rtype in (1, 28) and size != (4 if rtype == 1 else 16):
                raise DNSInvalid("invalid address RDATA")
            elif rtype == 41:
                if section != 2 or owner or any(r[2] == 41 for r in records):
                    raise DNSInvalid("invalid OPT placement")
                udp_size = max(512, min(rclass, MAX_MESSAGE))
                option = start
                while option < end:
                    if option + 4 > end:
                        raise DNSInvalid("short EDNS option")
                    option += 4 + struct.unpack_from("!H", data, option + 2)[0]
                    if option > end:
                        raise DNSInvalid("short EDNS payload")
            records.append((ttl_pos, ttl, rtype, section))
            pos = end
    if pos != len(data):
        raise DNSInvalid("trailing DNS bytes")
    return Message(ident, flags, (name, kind, cls), question_end, records, udp_size)


def question(name, kind=6, ident=None):
    return struct.pack("!6H", secrets.randbelow(65536) if ident is None else ident,
                       0x100, 1, 0, 0, 0) + wire_name(name) + struct.pack("!HH", kind, 1)


def reply_error(data, parsed, rcode=2, truncated=False):
    # Re-encode the question: incoming compression can refer to omitted bytes.
    name, kind, cls = parsed.question
    q = b"".join(bytes([len(label)]) + label for label in name) + b"\0"
    flags = 0x8080 | (parsed.flags & 0x0100) | rcode
    if truncated:
        flags |= 0x0200
    return struct.pack("!6H", parsed.ident, flags, 1, 0, 0, 0) + q + struct.pack("!HH", kind, cls)


def reverse_address(labels):
    try:
        if labels[-2:] == (b"in-addr", b"arpa") and len(labels) == 6:
            pieces = [p.decode("ascii") for p in reversed(labels[:4])]
            if any(not p.isdecimal() or (len(p) > 1 and p[0] == "0") for p in pieces):
                return None
            return ipaddress.IPv4Address(".".join(pieces))
        if labels[-2:] == (b"ip6", b"arpa") and len(labels) == 34:
            digits = b"".join(reversed(labels[:32])).decode("ascii")
            if any(len(p) != 1 for p in labels[:32]) or any(c not in "0123456789abcdef" for c in digits):
                return None
            return ipaddress.IPv6Address(int(digits, 16))
    except (ValueError, UnicodeError):
        return None
    return None


class Router:
    def __init__(self, zone, previous_zones, prefixes, nameservice, upstreams):
        self.zone = tuple(p.encode("ascii") for p in zone.split("."))
        self.previous = [tuple(p.encode("ascii") for p in z.split("."))
                         for z in previous_zones if z != zone]
        self.prefixes = prefixes
        self.nameservice, self.upstreams = nameservice, upstreams

    @staticmethod
    def suffix(name, zone):
        return len(name) >= len(zone) and name[-len(zone):] == zone

    def route(self, name):
        # glibc can retry a bare name after a search-domain failure. Reserve
        # single labels for Mesh short-name intent; never leak that retry.
        if len(name) == 1:
            return []
        if self.suffix(name, self.zone):
            return [self.nameservice]
        if any(self.suffix(name, z) for z in self.previous):
            return []  # Retired local zones must not escape to external DNS.
        address = reverse_address(name)
        if address is not None and any(address.version == p.version and address in p for p in self.prefixes):
            return [self.nameservice]
        return self.upstreams


class Exchange(asyncio.DatagramProtocol):
    def __init__(self, request, parsed, future):
        self.request, self.parsed, self.future = request, parsed, future

    def connection_made(self, transport):
        self.transport = transport
        transport.sendto(self.request)

    def datagram_received(self, data, addr):
        # Connected UDP filters the peer in the kernel; also verify explicitly.
        peer = self.transport.get_extra_info("peername")
        if addr[:2] != peer[:2]:
            return
        try:
            valid_response(data, self.parsed)
        except DNSInvalid:
            return
        if not self.future.done():
            self.future.set_result(data)

    def error_received(self, exc):
        if not self.future.done():
            self.future.set_exception(exc)


def valid_response(data, request):
    response = parse(data)
    if (response.ident != request.ident or not response.flags & 0x8000
            or (response.flags & 0x7800) != (request.flags & 0x7800)
            or response.question != request.question):
        raise DNSInvalid("response does not match transaction")
    return response


async def tcp_exchange(endpoint, request, parsed):
    reader, writer = await asyncio.open_connection(*endpoint, limit=MAX_MESSAGE + 2)
    try:
        writer.write(struct.pack("!H", len(request)) + request)
        await writer.drain()
        size = struct.unpack("!H", await reader.readexactly(2))[0]
        if not 12 <= size <= MAX_MESSAGE:
            raise DNSInvalid("TCP DNS frame outside bounds")
        data = await reader.readexactly(size)
        valid_response(data, parsed)
        return data
    finally:
        writer.close()
        # No unbounded wait_closed; this socket has no TLS shutdown handshake.


async def forward(endpoint, data):
    request = struct.pack("!H", secrets.randbelow(65536)) + data[2:]
    parsed = parse(request)
    loop = asyncio.get_running_loop()
    future = loop.create_future()
    transport = None
    try:
        transport, _ = await loop.create_datagram_endpoint(
            lambda: Exchange(request, parsed, future), remote_addr=endpoint,
            family=socket.AF_INET6 if ":" in endpoint[0] else socket.AF_INET)
        response = await future
        if parse(response).flags & 0x0200:
            response = await tcp_exchange(endpoint, request, parsed)
            if parse(response).flags & 0x0200:
                raise DNSInvalid("TCP response is still truncated")
        return response
    finally:
        if transport is not None:
            transport.close()


def age_response(response, request, elapsed):
    parsed = parse(response)
    positive = [ttl for _, ttl, kind, section in parsed.records if kind != 41 and section == 0]
    if positive and elapsed >= min(positive):
        return reply_error(b"", request)
    result = bytearray(response)
    struct.pack_into("!H", result, 0, request.ident)
    # Round upward so proxy time never extends a received TTL/lease.
    seconds = math.ceil(elapsed)
    for offset, ttl, kind, _ in parsed.records:
        if kind != 41:
            struct.pack_into("!I", result, offset, max(0, ttl - seconds))
    return bytes(result)


class Proxy(asyncio.DatagramProtocol):
    def __init__(self, router):
        self.router = router
        self.tasks = set()
        self.writers = set()
        self.transport = self.server = None

    async def bind(self, endpoint):
        loop = asyncio.get_running_loop()
        try:
            self.transport, _ = await loop.create_datagram_endpoint(lambda: self, local_addr=endpoint)
            actual = self.transport.get_extra_info("sockname")[:2]
            self.server = await asyncio.start_server(self.tcp_client, *actual, limit=MAX_MESSAGE + 2)
            return actual
        except BaseException:
            await self.close()
            raise

    def connection_made(self, transport):
        self.transport = transport

    def spawn(self, coro):
        task = asyncio.create_task(coro)
        self.tasks.add(task)
        task.add_done_callback(self.tasks.discard)

    def datagram_received(self, data, addr):
        if len(self.tasks) < MAX_CONCURRENT:
            self.spawn(self.udp_client(data, addr))

    async def answer(self, data):
        request = parse(data)
        if request.flags & 0x8000 or request.flags & 0x7800 or request.question[2] != 1:
            return reply_error(data, request, 5)
        if request.question[1] in (251, 252, 255):
            return reply_error(data, request, 5)
        if any(kind != 41 for _, _, kind, _ in request.records):
            return reply_error(data, request, 5)
        endpoints = self.router.route(request.question[0])
        if not endpoints:
            return reply_error(data, request, 5)
        started = time.monotonic()
        # One total budget, including retries. Rotate externally on failures only;
        # DNS error responses are authoritative for this transaction.
        for index, endpoint in enumerate(endpoints):
            remaining = DEADLINE - (time.monotonic() - started)
            if remaining <= 0:
                break
            try:
                async with asyncio.timeout(remaining / (len(endpoints) - index)):
                    response = await forward(endpoint, data)
                return age_response(response, request, time.monotonic() - started)
            except (OSError, TimeoutError, DNSInvalid, asyncio.IncompleteReadError):
                continue
        return reply_error(data, request)

    async def udp_client(self, data, addr):
        try:
            parsed = parse(data)
            response = await self.answer(data)
            if len(response) > parsed.udp_size:
                response = reply_error(data, parsed, 0, truncated=True)
            self.transport.sendto(response, addr)
        except (DNSInvalid, OSError):
            pass

    def tcp_client(self, reader, writer):
        if len(self.tasks) >= MAX_CONCURRENT:
            writer.close()
            return
        self.writers.add(writer)
        self.spawn(self.tcp_session(reader, writer))

    async def tcp_session(self, reader, writer):
        try:
            async with asyncio.timeout(DEADLINE):
                size = struct.unpack("!H", await reader.readexactly(2))[0]
                if not 12 <= size <= MAX_MESSAGE:
                    return
                data = await reader.readexactly(size)
            # answer has its own two-second total upstream budget and emits
            # SERVFAIL on expiry. Do not cancel it with the framing deadline.
            response = await self.answer(data)
            writer.write(struct.pack("!H", len(response)) + response)
            async with asyncio.timeout(0.5):
                await writer.drain()
        except (DNSInvalid, OSError, TimeoutError, asyncio.IncompleteReadError):
            pass
        finally:
            writer.close()
            self.writers.discard(writer)

    async def close(self):
        if self.transport:
            self.transport.close()
        if self.server:
            self.server.close()
            await self.server.wait_closed()
        for writer in self.writers:
            writer.close()
        tasks = list(self.tasks)
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        # DatagramTransport.close schedules descriptor release on the loop.
        # Complete that turn before reporting rollback/reusing the port.
        await asyncio.sleep(0)
