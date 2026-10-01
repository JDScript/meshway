#!/usr/bin/env python3
"""Host-only parser/fixture tests: no Docker and no fake DNS listener."""
import importlib.util
import ipaddress
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from wire import name_at, parse, receive_exact, request

NAME = "solo.fixture.meshway.test"
ID = 321


def reply(flags=0x8500, rtype=1, value=b"\x64\x60\x00\x01", ttl=9, records=1):
    question = request(NAME, "A", ID)[12:]
    rr = b"\xc0\x0c" + struct.pack("!HHIH", rtype, 1, ttl, len(value)) + value
    return struct.pack("!6H", ID, flags, 1, records, 0, 0) + question + (rr if records else b"")


class WireTests(unittest.TestCase):
    def test_transaction_question_owner_and_ttl(self):
        value = parse(reply(), NAME.upper() + ".", "A", ID)
        self.assertEqual(value["rcode"], "NOERROR")
        self.assertFalse(value["ra"])
        self.assertTrue(value["aa"])
        self.assertEqual(value["transaction_id"], ID)
        self.assertEqual(value["answers"], [{"name": NAME, "type": 1, "class": 1, "ttl": 9, "value": "100.96.0.1"}])

    def test_no_records_has_real_failure_header(self):
        for code, name in ((2, "SERVFAIL"), (3, "NXDOMAIN"), (5, "REFUSED")):
            value = parse(reply(flags=0x8100 | code, records=0), NAME, "A", ID)
            self.assertEqual(value["rcode"], name)
            self.assertEqual(value["answers"], [])
        with self.assertRaises(ValueError):
            parse(b"", NAME, "A", ID)

    def test_rejects_mismatch_truncation_cycles_and_trailing_data(self):
        for packet, name, kind, identifier in ((reply(), NAME, "A", ID + 1), (reply(), "other.test", "A", ID),
                                               (reply(), NAME, "AAAA", ID), (reply(flags=0x0100), NAME, "A", ID),
                                               (reply()[:-1], NAME, "A", ID), (reply() + b"x", NAME, "A", ID)):
            with self.assertRaises(ValueError):
                parse(packet, name, kind, identifier)
        for packet in (b"\xc0\x00", b"\xc0", b"\x40bad", b"\x03x"):
            with self.assertRaises(ValueError):
                name_at(packet, 0)

    def test_ipv6_and_ptr_parse_without_text_output_assumptions(self):
        ipv6 = ipaddress.IPv6Address("fd00::123")
        value = parse(reply(rtype=28, value=ipv6.packed), NAME, "A", ID)
        self.assertEqual(value["answers"][0]["value"], str(ipv6))
        value = parse(reply(rtype=12, value=b"\xc0\x0c"), NAME, "A", ID)
        self.assertEqual(value["answers"][0]["value"], NAME)
        with self.assertRaises(ValueError):
            parse(reply(rtype=12, value=b"\xc0\x0c\0"), NAME, "A", ID)

    def test_tcp_exact_read_fragmentation_and_eof(self):
        class Reader:
            def __init__(self, values):
                self.values = iter(values)

            def recv(self, _size):
                return next(self.values)

        self.assertEqual(receive_exact(Reader([b"a", b"bc"]), 3), b"abc")
        with self.assertRaises(ValueError):
            receive_exact(Reader([b"a", b""]), 3)

    def test_request_bounds(self):
        for name in ("a" * 64 + ".test", "x..test", ".", ".".join(["a" * 63] * 5)):
            with self.assertRaises(ValueError):
                request(name, "A", ID)


class PrivateFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("dns_fixture", HERE / "container.py")
        cls.fixture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.fixture)

    def test_enrollment_persists_private_binding_before_sdk_and_never_replaces_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(self.fixture, "ROOT", root), patch.object(self.fixture, "STATE", root / "agent.json"):
                import hashlib
                import json
                value = self.fixture.main("prepare-enrollment", {"name": "Twin", "token": "secret-grant"})
                state = json.loads((root / "agent.json").read_text())
                self.assertEqual(state["node_credential"], value["credential"])
                self.assertIsNone(state["node_id"])
                self.assertEqual(state["enrollment_token_hash"], list(hashlib.sha256(b"secret-grant").digest()))
                self.assertNotIn("secret-grant", (root / "agent.json").read_text())
                self.assertEqual((root / "agent.json").stat().st_mode & 0o777, 0o600)
                with self.assertRaises(ValueError):
                    self.fixture.main("prepare-enrollment", {"name": "Twin", "token": "other"})

    def test_agent_cli_uses_same_state_explicit_dns_and_coexists_without_secret_argv(self):
        with patch.object(self.fixture, "start", return_value={"started": True}) as start:
            self.fixture.main("agent-start", {"dataplane": True})
            command = start.call_args.args[1]
            self.assertEqual(command[:2], ["meshway-agent", "run"])
            self.assertIn("--apply-dataplane", command)
            self.assertEqual(command[command.index("--state") + 1], str(self.fixture.STATE))
            self.assertEqual(command[command.index("--dns-listen") + 1], "127.0.0.1:5353")
            self.assertNotIn("--token", command)
            self.assertNotIn("--connections", command)
        with patch.object(self.fixture, "start", return_value={"started": True}) as start:
            self.fixture.main("agent-start", {})
            self.assertNotIn("--apply-dataplane", start.call_args.args[1])


if __name__ == "__main__":
    unittest.main()
