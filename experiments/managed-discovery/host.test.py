#!/usr/bin/env python3
"""Host-only bounded wire/guard/ledger and process-FD inventory acceptance tests."""
import importlib.util
from pathlib import Path
import socket
import struct
import sys
import threading
import unittest

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


wire = load("stun_wire", HERE.parent / "netcheck/wire.py")
load("wire", HERE.parent / "managed-dns/wire.py")
load("base", HERE.parent / "managed-dns/container.py")
observer = load("discovery_observer", HERE / "observer.py")
container = load("discovery_container", HERE / "container.py")


class ObserverTests(unittest.TestCase):
    def setUp(self):
        self.ledger = observer.Ledger(["172.23.0.3", "172.23.0.4"])
        self.request = wire.binding_request(bytes(range(12)))

    def test_two_sources_actual_peer_address_and_port(self):
        for peer in [("172.23.0.3", 41000), ("172.23.0.3", 45231), ("172.23.0.4", 41000)]:
            response = self.ledger.respond(self.request, peer, 0, 123)
            self.assertEqual(wire.mapped_response(response, bytes(range(12))), f"{peer[0]}:{peer[1]}")
        self.assertEqual(self.ledger.value["count"], 3)
        self.assertEqual([item["port"] for item in self.ledger.value["peers"]], [41000, 45231, 41000])

    def test_source_guard_bad_frames_and_packet_cap_do_not_create_evidence(self):
        for packet, peer in [(self.request, ("172.23.0.5", 41000)), (b"x" * 1025, ("172.23.0.3", 41000)),
                             (self.request[:-1], ("172.23.0.3", 41000)), (b"\x00" * 20, ("172.23.0.3", 41000))]:
            self.assertIsNone(self.ledger.respond(packet, peer, 0, 123))
        self.assertEqual(self.ledger.value, {"count": 0, "peers": []})
        self.ledger = observer.Ledger(["172.23.0.3", "172.23.0.4"])
        peer = ("172.23.0.3", 41000)
        for _ in range(20):
            self.assertIsNotNone(self.ledger.respond(self.request, peer, 0, 123))
        self.assertIsNone(self.ledger.respond(self.request, peer, 0, 124))
        self.assertEqual(self.ledger.value["count"], 20)
        self.assertEqual(self.ledger.value["peers"][0]["last_at"], 123)
        self.assertIsNotNone(self.ledger.respond(self.request, peer, 1, 125))
        self.assertEqual(self.ledger.value["peers"][0]["last_at"], 125)

    def test_only_explicit_private_numeric_bridge_addresses(self):
        for address in ["observer", "127.0.0.1", "0.0.0.0", "8.8.8.8", "224.0.0.1", "169.254.1.2"]:
            with self.assertRaises(ValueError):
                observer.private_ip(address)
        with self.assertRaises(ValueError):
            observer.Ledger(["172.23.0.3", "172.23.0.3"])

    def test_restoration_keeps_exact_peer_counters_without_synthetic_events(self):
        peer = ("172.23.0.3", 41000)
        self.ledger.respond(self.request, peer, 0, 100)
        restored = observer.Ledger(["172.23.0.3", "172.23.0.4"], self.ledger.value)
        self.assertEqual(restored.value["count"], 1)
        restored.respond(self.request, peer, 1, 200)
        self.assertEqual(restored.value["peers"][0], {"ip": peer[0], "port": peer[1], "count": 2, "first_at": 100, "last_at": 200})

    def test_ipv6_wire_xor_and_transaction_mismatch(self):
        response = wire.binding_response(self.request, ("fd00::1", 45000))
        self.assertEqual(wire.mapped_response(response, bytes(range(12))), "[fd00::1]:45000")
        with self.assertRaises(ValueError):
            wire.mapped_response(response, b"x" * 12)
        # Comprehension-required attributes cannot elicit a successful mapping.
        bad = wire.HEADER.pack(1, 4, wire.COOKIE, bytes(range(12))) + struct.pack("!HH", 1, 0)
        self.assertIsNone(self.ledger.respond(bad, ("172.23.0.3", 41000), 0, 123))

    def test_real_loopback_wire_roundtrip_reflects_recvfrom_source(self):
        # Bounded wire test; acceptance observer itself never permits loopback.
        errors = []
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            server.bind(("127.0.0.1", 0)); server.settimeout(2)
            client.bind(("127.0.0.1", 0)); client.settimeout(2)

            def serve():
                try:
                    request, peer = server.recvfrom(1025)
                    server.sendto(wire.binding_response(request, peer), peer)
                except Exception as error:
                    errors.append(error)

            thread = threading.Thread(target=serve)
            thread.start()
            try:
                client.sendto(self.request, server.getsockname())
                response, _peer = client.recvfrom(1025)
                self.assertEqual(wire.mapped_response(response, bytes(range(12))), f"127.0.0.1:{client.getsockname()[1]}")
            finally:
                thread.join(3)
            self.assertFalse(thread.is_alive()); self.assertEqual(errors, [])


class InventoryTests(unittest.TestCase):
    def test_inventory_only_counts_agent_owned_socket_inodes(self):
        rows = "header\n0: 00000000:A028 00000000:0000 07 0:0 0:0 0 0 0 111\n1: 0100007F:14E9 00000000:0000 07 0:0 0:0 0 0 0 222\n"
        self.assertEqual(container.parse_udp_table(rows, {"111": 2}, 4),
                         [{"family": 4, "host": "0.0.0.0", "port": 41000, "inode": "111", "fd_count": 2}])
        self.assertEqual(container.parse_udp_table(rows, {}, 4), [])
        result = container.parse_udp_table(rows, {"222": 1}, 4)
        self.assertEqual((result[0]["host"], result[0]["port"]), ("127.0.0.1", 5353))

    def test_ipv6_word_endianness(self):
        rows = "header\n0: 00000000000000000000000001000000:A028 0:0 07 0:0 0:0 0 0 0 111\n"
        self.assertEqual(container.parse_udp_table(rows, {"111": 1}, 6)[0]["host"], "::1")


if __name__ == "__main__":
    unittest.main()
