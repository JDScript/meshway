"""Host-only tests of the untrusted pcap reader; no network/conformance simulation."""
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("traversal_evidence", Path(__file__).with_name("evidence.py"))
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)


def capture(payload, *, fragment=0, order="<", udp_length=None):
    length = 8 + len(payload) if udp_length is None else udp_length
    ipv4 = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 28 + len(payload), 0, fragment, 64, 17, 0,
                       b"\xc0\x00\x02\x01", b"\xc0\x00\x02\x02")
    frame = bytes(12) + b"\x08\x00" + ipv4 + struct.pack("!HHHH", 35000, 3478, length, 0) + payload
    return (struct.pack(order + "IHHIIII", 0xa1b2c3d4, 2, 4, 0, 0, 128, 1)
            + struct.pack(order + "IIII", 1, 2, len(frame), len(frame)) + frame)


def tcp_capture(payload=b"", *, flags=0x10, data_offset=5):
    ip = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 40 + len(payload), 0, 0, 64, 6, 0,
                     b"\xc0\x00\x02\x01", b"\xc0\x00\x02\x02")
    tcp = struct.pack("!HHIIBBHHH", 41000, 35001, 1, 1, data_offset << 4, flags, 4096, 0, 0)
    frame = bytes(12) + b"\x08\x00" + ip + tcp + payload
    return (struct.pack("<IHHIIII", 0xa1b2c3d4, 2, 4, 0, 0, 128, 1)
            + struct.pack("<IIII", 2, 3, len(frame), len(frame)) + frame)


class CaptureReader(unittest.TestCase):
    def read(self, data):
        with tempfile.TemporaryDirectory(prefix="meshway-pcap-parser-") as directory:
            path = Path(directory) / "bounded.pcap"
            path.write_bytes(data)
            return list(evidence.packets(path))

    def test_endianness_and_public_discriminators(self):
        cases = [(b"\x00\x01\x00\x00\x21\x12\xa4\x42" + bytes(12), "stunRequest"),
                 (b"MWADJv1\0\x03\x00\x01x", "adjacencyAck"),
                 (b"MWCIR\0\0\x04", "meshCiphertext")]
        for order in ("<", ">"):
            for payload, kind in cases:
                with self.subTest(order=order, kind=kind):
                    self.assertEqual(self.read(capture(payload, order=order)),
                                     [("192.0.2.1", 35000, "192.0.2.2", 3478, kind, 1_000_002, None)])

    def test_actual_offer_bound_traversal_subtypes(self):
        digest = bytes(range(32))
        for subtype, kind in enumerate(("coordinationPunch", "coordinationInit", "coordinationChallenge",
                                         "coordinationAck", "coordinationReady")):
            for order in ("<", ">"):
                with self.subTest(subtype=subtype, order=order):
                    packet = self.read(capture(b"MWTRV1\0\0" + digest + bytes([subtype]) + b"body", order=order))[0]
                    self.assertEqual(packet.kind, kind)
                    self.assertEqual(packet.offer_digest, digest.hex())

    def test_truncated_or_unknown_traversal_header_fails_closed(self):
        for payload in (b"MWTRV1\0\0", b"MWTRV1\0\0" + bytes(31), b"MWTRV1\0\0" + bytes(32),
                        b"MWTRV1\0\0" + bytes(32) + b"\x05", b"MWTRV1\0\0" + bytes(32) + b"\xff"):
            with self.subTest(payload=payload.hex()), self.assertRaisesRegex(RuntimeError, "traversal envelope"):
                self.read(capture(payload))

    def test_truncation_and_forged_record_sizes_fail_closed(self):
        valid = capture(b"payload")
        for data in (valid[:10], valid[:-1], valid[:24] + bytes(15),
                     valid[:32] + struct.pack("<I", 129) + valid[36:]):
            with self.subTest(size=len(data)), self.assertRaises(RuntimeError):
                self.read(data)

    def test_fragment_and_inconsistent_udp_length_fail_closed(self):
        for data in (capture(b"payload", fragment=0x2000), capture(b"payload", fragment=1),
                     capture(b"payload", udp_length=8)):
            with self.assertRaises(RuntimeError):
                self.read(data)

    def test_unknown_payload_is_not_mesh_or_handshake_proof(self):
        self.assertEqual(self.read(capture(b"MWCIR-not-the-wire-version"))[0][4], "other")
        self.assertEqual(self.read(capture(b"MWCIR\0\0\x02"))[0][4], "other")
        self.assertEqual(self.read(capture(b"MWCIR\0\0\x03"))[0][4], "other")

    def test_tcp_handshake_is_distinct_from_bidirectional_data(self):
        for flags, payload, kind in ((2, b"", "tcpSyn"), (0x12, b"", "tcpSynAck"),
                                     (0x14, b"", "tcpReset"), (0x10, b"", "tcpAck"),
                                     (0x18, b"opaque stream fragment", "tcpData")):
            with self.subTest(kind=kind):
                self.assertEqual(self.read(tcp_capture(payload, flags=flags))[0],
                                 ("192.0.2.1", 41000, "192.0.2.2", 35001, kind, 2_000_003, None))

    def test_malformed_tcp_data_offsets_fail_closed(self):
        for offset in (0, 4, 15):
            with self.subTest(offset=offset), self.assertRaises(RuntimeError):
                self.read(tcp_capture(data_offset=offset))

    def test_stun_only_is_not_a_traversal_pass(self):
        with tempfile.TemporaryDirectory(prefix="meshway-pcap-parser-") as directory:
            report = Path(directory)
            router = report / "router-a"
            router.mkdir()
            (report / "controller").mkdir()
            (report / "controller/traversal-verified-offers.json").write_text("[]\n")
            (router / "private.pcap").write_bytes(capture(b"\x00\x01\x00\x00\x21\x12\xa4\x42" + bytes(12)))
            (router / "private-capture.stderr").write_text("0 packets dropped by kernel\n")
            with self.assertRaisesRegex(RuntimeError, "Missing complete same-offer"):
                evidence.verify(report, {"a": "192.0.2.1", "controller": "192.0.2.2"},
                                {"a": {"private": "192.0.2.3", "wan": "198.51.100.1"},
                                 "r": {"private": "192.0.2.4", "wan": "198.51.100.2"}})


class FirewallEvidence(unittest.TestCase):
    dns = ("[0:0] -A OUTPUT -d 127.0.0.11/32 -j DOCKER_OUTPUT\n"
           "[0:0] -A DOCKER_OUTPUT -d 127.0.0.11/32 -p udp -m udp --dport 53 "
           "-j DNAT --to-destination 127.0.0.11:33811\n")

    def test_only_unchanged_output_only_embedded_dns_is_excluded(self):
        self.assertEqual(evidence.forwarding_dnat(self.dns.replace("[0:0]", "[3:240]"), self.dns), [])
        changed = (self.dns.replace("33811", "33812"),
                   self.dns.replace("--dport 53", "--dport 35000"),
                   self.dns.replace("127.0.0.11:33811", "192.0.2.1:35000"))
        for current in changed:
            with self.subTest(current=current):
                self.assertEqual(len(evidence.forwarding_dnat(current, self.dns)), 1)
        self.assertEqual(len(evidence.forwarding_dnat(self.dns, "")), 1)

    def test_docker_chain_name_does_not_hide_forwarding_hook(self):
        for hook in ("-A PREROUTING -j DOCKER_OUTPUT", "-A OTHER -g DOCKER_OUTPUT"):
            current = self.dns + "[0:0] " + hook + "\n"
            for baseline in (self.dns, current):
                with self.subTest(hook=hook, baseline=baseline):
                    self.assertEqual(len(evidence.forwarding_dnat(current, baseline)), 1)

    def test_udp_or_tcp_forwarding_retains_real_counter_with_local_dns(self):
        for protocol in ("udp", "tcp"):
            current = self.dns + (f"[4:280] -A PREROUTING -d 198.51.100.2/32 -p {protocol} "
                                  "--dport 35001 -j DNAT --to-destination 192.0.2.1:35001\n")
            [(count, options)] = evidence.forwarding_dnat(current, self.dns)
            self.assertEqual(count, 4)
            self.assertEqual(options["-p"], protocol)
            self.assertEqual(options["-A"], "PREROUTING")

    def test_missing_kernel_counter_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "kernel counter"):
            evidence.forwarding_dnat(self.dns.replace("[0:0] ", ""), self.dns)


class HandshakeCorrelation(unittest.TestCase):
    local = "192.0.2.1"
    public = "198.51.100.1"
    peer = "198.51.100.2"
    digest = "12" * 32
    other_digest = "34" * 32

    def offers(self):
        return [{"digest": digest, "local": {"address": self.public + ":35000"},
                 "peer": {"address": self.peer + ":35000"}} for digest in (self.digest, self.other_digest)]

    def records(self):
        return [evidence.Packet(source, 35000, destination, 35000, kind, time, self.digest if time <= 4 else None)
                for source, destination, kind, time in (
                    (self.local, self.peer, "coordinationInit", 1),
                    (self.peer, self.local, "coordinationChallenge", 2),
                    (self.local, self.peer, "coordinationAck", 3),
                    (self.peer, self.local, "coordinationReady", 4),
                    (self.local, self.peer, "meshCiphertext", 5),
                    (self.peer, self.local, "meshCiphertext", 6))]

    def verify(self, records, offers=None):
        return evidence.handshake(records, self.local, self.peer, self.public, self.offers() if offers is None else offers)

    def test_complete_same_digest_bilateral_handshake_then_ciphertext(self):
        observed = self.verify(self.records())
        self.assertEqual(observed["offerDigest"], self.digest)
        self.assertEqual(observed["phaseUnixUs"]["coordinationReady"], 4)
        self.assertEqual(observed["ciphertextUnixUs"], {"outbound": 5, "inbound": 6})

    def test_cross_digest_phases_cannot_be_combined(self):
        records = self.records()
        records[2] = records[2]._replace(offer_digest=self.other_digest)
        records[3] = records[3]._replace(offer_digest=self.other_digest)
        with self.assertRaisesRegex(RuntimeError, "same-offer"):
            self.verify(records)

    def test_wrong_direction_endpoint_order_or_missing_ciphertext_rejected(self):
        original = self.records()
        corruptions = (original[:1] + [original[1]._replace(source=self.local, destination=self.peer)] + original[2:],
                       [original[0]._replace(source_port=35002)] + original[1:],
                       original[:3] + [original[3]._replace(timestamp_us=2)] + original[4:],
                       original[:4], original[:5],
                       original[:4] + [original[4]._replace(timestamp_us=0), original[5]._replace(timestamp_us=0)])
        for records in corruptions:
            with self.subTest(records=records), self.assertRaisesRegex(RuntimeError, "same-offer"):
                self.verify(records)

    def test_packet_digest_must_match_verified_http_offer_and_peer_pair(self):
        wrong_pair = self.offers()
        wrong_pair[0]["peer"]["address"] = "198.51.100.99:35000"
        for offers in ([], self.offers()[1:], wrong_pair):
            with self.subTest(offers=json.dumps(offers)), self.assertRaisesRegex(RuntimeError, "same-offer"):
                self.verify(self.records(), offers)


if __name__ == "__main__":
    unittest.main()
