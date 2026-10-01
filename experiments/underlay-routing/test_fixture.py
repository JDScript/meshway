"""Host-only boundaries and wire parsing; no Docker or privileges required."""
import json
from pathlib import Path
import socket
import struct
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from helper import Reservation, RouteGuard, fresh, parse_packet, parse_stun, stun_reply, stun_request
from run import LABEL, owns


class Boundaries(unittest.TestCase):
    def test_conflicts_reject_without_adoption(self):
        reservation = Reservation()
        for rules, tables, marks in (
            ([{'priority': 18000, 'table': 254}], set(), ()),
            ([{'priority': 18003, 'table': 254}], set(), ()),
            ([{'priority': 900, 'table': 18402}], set(), ()),
            ([{'priority': 900, 'table': '18402'}], set(), ()),
            ([], {18401}, ()),
            ([{'priority': 900, 'fwmark': '0x10000/0xffff0000'}], set(), ()),
            ([], set(), ((0, 0x10000),)),
            ([{'priority': 900, 'uidrange': '1000-1000'}], set(), ()),
        ):
            with self.subTest(rules=rules, tables=tables, marks=marks), self.assertRaises(ValueError):
                reservation.validate(rules, tables, marks)

    def test_existing_disjoint_policy_accepted(self):
        Reservation().validate([{'priority': 19000, 'fwmark': '0x20', 'fwmask': '0xff', 'table': 18550}], {18550})

    def test_invalid_reservations(self):
        for reservation in (Reservation(mark=0), Reservation(mask=0), Reservation(mark=1),
                            Reservation(priorities=(18000, 18000)), Reservation(tables=(254, 18402))):
            with self.subTest(reservation=reservation), self.assertRaises(ValueError):
                reservation.validate([], set())
        with self.assertRaises(ValueError):
            Reservation(priorities=(18000, 18002, 18004, 18006)).validate([], set())

    def test_no_path_no_socket_no_business_purpose(self):
        guard = RouteGuard(Reservation(), 1)
        with patch('helper.socket.socket') as factory:
            with self.assertRaises(RuntimeError):
                guard.socket('stun', socket.AF_INET, socket.SOCK_DGRAM)
            guard.prepare([], set())
            guard.activate()
            with self.assertRaises(RuntimeError):
                guard.socket('business', socket.AF_INET, socket.SOCK_STREAM)
            with self.assertRaises(RuntimeError):
                guard.socket('stun', socket.AF_INET, socket.SOCK_DGRAM, existing_mark=0x4D570000)
            with self.assertRaises(RuntimeError):
                guard.socket('stun', socket.AF_INET, socket.SOCK_DGRAM, existing_mark=0x100000000)
            factory.assert_not_called()

    def test_permission_denied_closes_and_never_retries_unmarked(self):
        guard = RouteGuard(Reservation(), 1)
        guard.prepare([], set())
        guard.activate()
        with patch('helper.socket.socket') as factory, patch('helper.socket.SO_MARK', 36, create=True):
            factory.return_value.setsockopt.side_effect = PermissionError('denied')
            with self.assertRaises(PermissionError):
                guard.socket('stun', socket.AF_INET, socket.SOCK_DGRAM)
            self.assertEqual(factory.call_count, 1)
            factory.return_value.close.assert_called_once()
            factory.return_value.sendto.assert_not_called()
            factory.return_value.connect.assert_not_called()
            self.assertEqual(guard.sockets, [])

    def test_mark_preserves_foreign_bits_and_precedes_bind(self):
        guard = RouteGuard(Reservation(), 1)
        guard.prepare([], set())
        guard.activate()
        with patch('helper.socket.socket') as factory, patch('helper.socket.SO_MARK', 36, create=True):
            factory.return_value.getsockopt.return_value = 0x4D570020
            sock = guard.socket('carrier', socket.AF_INET, socket.SOCK_STREAM,
                                source='172.30.247.10', existing_mark=0x20)
            calls = sock.method_calls
            self.assertEqual(calls[0].args, (socket.SOL_SOCKET, 36, 0x4D570020))
            self.assertEqual(calls[-1].args, (('172.30.247.10', 0),))
            sock.connect.assert_not_called()
            guard.retire()

    def test_generation_and_expiry_not_refreshed(self):
        guard = RouteGuard(Reservation(), 2)
        guard.prepare([], set())
        guard.activate()
        observation = {'generation': 2, 'observed_at': 10, 'expires_at': 20}
        self.assertTrue(fresh(observation, guard, 11))
        self.assertFalse(fresh(observation, guard, 9))
        self.assertFalse(fresh(observation, guard, 20))
        self.assertFalse(fresh({**observation, 'generation': 1}, guard, 11))
        guard.retire()
        self.assertFalse(fresh(observation, guard, 11))

    def test_precise_resource_ownership(self):
        self.assertTrue(owns([{'Config': {'Labels': {LABEL: 'mine'}}}], 'mine'))
        self.assertTrue(owns([{'Labels': {LABEL: 'mine'}}], 'mine', network=True))
        for value in (None, [], [{'Config': {'Labels': {LABEL: 'other'}}}],
                      [{'Config': {'Labels': None}}], [{'Config': None}], [{}], [{}, {}]):
            self.assertFalse(owns(value, 'mine'))


class Parsing(unittest.TestCase):
    def test_stun_real_wire_v4_v6(self):
        transaction = bytes(range(12))
        for address in ('172.30.247.10', 'fd74:6d77:247::10'):
            request = stun_request(transaction)
            reply = stun_reply(request, (address, 41863))
            self.assertEqual(parse_stun(reply, transaction), (address, 41863))
            with self.assertRaises(ValueError):
                parse_stun(reply, b'wrong-tx-123')
            with self.assertRaises(ValueError):
                parse_stun(reply[:-1], transaction)
        with self.assertRaises(ValueError):
            stun_reply(b'not STUN', ('172.30.247.10', 1))

    def test_tun_evidence_parser(self):
        packet = bytearray(28)
        packet[0], packet[9] = 0x45, 17
        packet[12:16], packet[16:20] = socket.inet_aton('172.30.247.10'), socket.inet_aton('172.30.247.20')
        packet[20:24] = struct.pack('!HH', 40000, 41863)
        evidence = parse_packet(packet)
        self.assertEqual(evidence['destination_port'], 41863)
        self.assertEqual(evidence['source'], '172.30.247.10')
        json.dumps(evidence)
        packet6 = bytearray(48)
        packet6[0], packet6[6] = 0x60, 6
        packet6[8:24], packet6[24:40] = socket.inet_pton(socket.AF_INET6, 'fd74:6d77:247::10'), socket.inet_pton(socket.AF_INET6, 'fd74:6d77:247::20')
        packet6[40:44] = struct.pack('!HH', 40000, 41862)
        self.assertEqual(parse_packet(packet6)['destination'], 'fd74:6d77:247::20')
        for malformed in (b'', b'\x45', b'\x60' + bytes(10), b'\x10'):
            with self.assertRaises(ValueError):
                parse_packet(malformed)


if __name__ == '__main__':
    unittest.main()
