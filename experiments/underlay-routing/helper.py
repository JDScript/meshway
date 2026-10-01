"""Experimental socket/route ownership contract; not a product API."""
from dataclasses import dataclass
import ipaddress
import socket
import struct
import time

COOKIE = 0x2112A442
PURPOSES = frozenset({'controller', 'stun', 'authenticated_probe', 'carrier', 'bootstrap_dns'})


@dataclass(frozen=True)
class Reservation:
    mark: int = 0x4D570000
    mask: int = 0xFFFF0000
    priorities: tuple = (18000, 18001, 18002, 18003)
    tables: tuple = (18401, 18402)

    def validate(self, rules, occupied_tables, foreign_marks=()):
        if not 0 < self.mark <= 0xFFFFFFFF or not 0 < self.mask <= 0xFFFFFFFF:
            raise ValueError('nonzero u32 mark/mask required')
        if self.mark & ~self.mask or len(set(self.priorities)) != 4 or len(set(self.tables)) != 2:
            raise ValueError('invalid reservation')
        if self.priorities != tuple(range(self.priorities[0], self.priorities[0] + 4)):
            raise ValueError('guard and captures require a contiguous priority block')
        if any(not 0 < p < 32766 for p in self.priorities) or any(t <= 255 for t in self.tables):
            raise ValueError('reserved priority/table')
        if set(self.tables) & set(occupied_tables):
            raise ValueError('foreign table owner')
        for rule in rules:
            if rule.get('priority') in self.priorities:
                raise ValueError('foreign priority owner')
            table = rule.get('table')
            if isinstance(table, str) and table.isdecimal():
                table = int(table)
            if table in self.tables:
                raise ValueError('foreign table reference')
            if 'fwmark' in rule:
                raw = rule['fwmark']
                if isinstance(raw, str) and '/' in raw:
                    value, mask = (int(x, 0) for x in raw.split('/'))
                else:
                    value = int(raw, 0) if isinstance(raw, str) else raw
                    mask = rule.get('fwmask', 0xFFFFFFFF)
                    mask = int(mask, 0) if isinstance(mask, str) else mask
                foreign_marks = (*foreign_marks, (value, mask))
            # Unknown RPDB selectors need explicit integration review; never silently adopt.
            supported = {'priority', 'src', 'dst', 'table', 'protocol', 'fwmark', 'fwmask', 'flags'}
            if set(rule) - supported:
                raise ValueError('unsupported RPDB selector')
        for value, mask in foreign_marks:
            # Refuse any overlapping reserved bits, even disjoint current values: ownership
            # of a mask is broader than one currently installed rule.
            if mask & self.mask:
                raise ValueError('foreign mark-mask owner')


class RouteGuard:
    def __init__(self, reservation, generation):
        self.reservation, self.generation = reservation, generation
        self.prepared, self.active, self.sockets = False, False, []

    def prepare(self, rules, occupied_tables, foreign_marks=()):
        self.reservation.validate(rules, occupied_tables, foreign_marks)
        self.prepared = True

    def activate(self):
        if not self.prepared:
            raise RuntimeError('no protected path')
        self.active = True

    def socket(self, purpose, family, kind, *, source=None, existing_mark=0):
        if purpose not in PURPOSES or not self.active:
            raise RuntimeError('unapproved purpose or inactive path')
        if not 0 <= existing_mark <= 0xFFFFFFFF or existing_mark & self.reservation.mask:
            raise RuntimeError('socket mark already owned')
        sock = socket.socket(family, kind)
        try:
            mark = existing_mark | self.reservation.mark
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_MARK, mark)
            if sock.getsockopt(socket.SOL_SOCKET, socket.SO_MARK) != mark:
                raise RuntimeError('mark not applied')
            sock.settimeout(.7)
            if source:
                sock.bind((source, 0))
        except BaseException:
            sock.close()
            raise  # No unmarked fallback.
        self.sockets.append(sock)
        return sock

    def retire(self):
        self.active = False
        for sock in self.sockets:
            sock.close()
        self.sockets.clear()


def stun_request(transaction):
    if len(transaction) != 12:
        raise ValueError('STUN transaction must be 96 bits')
    return struct.pack('!HHI', 1, 0, COOKIE) + transaction


def stun_reply(request, peer):
    if len(request) != 20 or request[:8] != struct.pack('!HHI', 1, 0, COOKIE):
        raise ValueError('not a standard binding request')
    address = ipaddress.ip_address(peer[0])
    key = struct.pack('!I', COOKIE) + request[8:20]
    packed = bytes(a ^ b for a, b in zip(address.packed, key))
    attr = struct.pack('!BBH', 0, 1 if address.version == 4 else 2, peer[1] ^ (COOKIE >> 16)) + packed
    return struct.pack('!HHI', 0x101, len(attr) + 4, COOKIE) + request[8:20] + struct.pack('!HH', 0x20, len(attr)) + attr


def parse_stun(reply, transaction):
    if len(reply) < 28:
        raise ValueError('truncated STUN')
    kind, size, cookie = struct.unpack('!HHI', reply[:8])
    if (kind, cookie, reply[8:20]) != (0x101, COOKIE, transaction) or size != len(reply) - 20:
        raise ValueError('wrong STUN transaction/header')
    attr, length = struct.unpack('!HH', reply[20:24])
    family = reply[25]
    expected = {1: 8, 2: 20}.get(family)
    if attr != 0x20 or length != expected or len(reply) != 24 + length:
        raise ValueError('invalid XOR-MAPPED-ADDRESS')
    key = struct.pack('!I', COOKIE) + transaction
    address = bytes(a ^ b for a, b in zip(reply[28:], key))
    return str(ipaddress.ip_address(address)), struct.unpack('!H', reply[26:28])[0] ^ (COOKIE >> 16)


def fresh(observation, guard, now=None):
    now = time.monotonic() if now is None else now
    return (guard.active and observation['generation'] == guard.generation
            and observation['observed_at'] <= now < observation['expires_at'])


def parse_packet(packet):
    if not packet:
        raise ValueError('empty TUN packet')
    version = packet[0] >> 4
    if version == 4:
        if len(packet) < 20 or (packet[0] & 15) < 5:
            raise ValueError('truncated IPv4')
        offset, proto = (packet[0] & 15) * 4, packet[9]
        source, destination = packet[12:16], packet[16:20]
    elif version == 6:
        if len(packet) < 40:
            raise ValueError('truncated IPv6')
        offset, proto = 40, packet[6]
        source, destination = packet[8:24], packet[24:40]
    else:
        raise ValueError('unknown IP version')
    if len(packet) < offset + 4 or proto not in (6, 17):
        raise ValueError('fixture expects TCP/UDP without extension headers')
    sport, dport = struct.unpack('!HH', packet[offset:offset + 4])
    return {'version': version, 'source': str(ipaddress.ip_address(bytes(source))),
            'destination': str(ipaddress.ip_address(bytes(destination))), 'protocol': proto,
            'source_port': sport, 'destination_port': dport}
