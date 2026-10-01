"""Host-only checks: real loopback UDP, strict reports, namespace plans and cleanup."""

from contextlib import contextmanager, nullcontext
from datetime import datetime, timezone
import ipaddress
import io
import fcntl
import json
import os
from pathlib import Path
import socket
import struct
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import zlib

import canary
from checks import CheckError, crosscheck, strict_json, validate_report
from kernel import SCENARIOS, router_commands
from observer import RATE_PER_SECOND, serve
import run
from wire import COOKIE, HEADER, MAX_REQUEST, binding_request, binding_response, endpoint, mapped_response

PARENT = run.TEMP if run.TEMP.is_dir() else Path(tempfile.gettempdir())
TRANSACTION = bytes(range(12))
SERVERS = ["192.0.2.10:3478", "192.0.2.11:3478"]
WAN = "192.0.2.1"


@contextmanager
def loopback_observer(version=4, allowed=None):
    address = "127.0.0.1" if version == 4 else "::1"
    family = socket.AF_INET if version == 4 else socket.AF_INET6
    listener = socket.socket(family, socket.SOCK_DGRAM)
    try:
        listener.bind((address, 0))
        with tempfile.TemporaryDirectory(prefix="netcheck-host-", dir=PARENT) as directory:
            snapshot = Path(directory) / "observer.json"
            stopped = threading.Event()
            errors = []

            def worker():
                try:
                    serve(listener, ipaddress.ip_network("127.0.0.0/8" if version == 4 else "::1/128"),
                          ipaddress.ip_address(allowed or address), snapshot, stopped)
                except Exception as error:
                    errors.append(error)

            thread = threading.Thread(target=worker)
            thread.start()
            try:
                yield listener.getsockname()[:2], snapshot
            finally:
                stopped.set()
                thread.join(timeout=2)
                if thread.is_alive():
                    raise AssertionError("observer thread did not terminate")
                if errors:
                    raise errors[0]
    finally:
        listener.close()


def report_for(ports=(41000, 41000, 41000)):
    stamp = lambda second: datetime.fromtimestamp(second, timezone.utc).isoformat().replace("+00:00", "Z")
    first, middle, last = ports
    stable = None if first is None or last is None else first == last
    varies = None if None in ports or not stable else first != middle
    samples = []
    for server, port in zip([SERVERS[0], SERVERS[1], SERVERS[0]], ports):
        samples.append({"server": server, "outcome": "timeout" if port is None else "mapped",
                        "mapped_endpoint": None if port is None else endpoint(WAN, port),
                        "rtt_ms": None if port is None else 1.2, "elapsed_ms": 700.0 if port is None else 1.2})
    return {"schema_version": 1, "source": "independent_udp_probe", "measured_at": stamp(1000),
            "completed_at": stamp(1003), "valid_until": stamp(1030), "bound_socket": "0.0.0.0:53000",
            "address_family": "ipv4", "udp_observed": any(port is not None for port in ports),
            "mapping_varies_by_destination": varies, "mapping_stable_during_check": stable, "samples": samples}


def validated(report):
    return validate_report(json.dumps(report), 0 if report["udp_observed"] else 1, SERVERS, 999, 1004)


class WireTests(unittest.TestCase):
    def test_real_peer_ipv4_and_ipv6_xor_encoding(self):
        for address in ("192.0.2.123", "2001:db8::123"):
            response = binding_response(binding_request(TRANSACTION), (address, 41001))
            self.assertEqual(mapped_response(response, TRANSACTION), endpoint(address, 41001))
            self.assertEqual(HEADER.unpack_from(response)[:3], (0x0101, len(response) - 20, COOKIE))

    def test_malformed_requests_never_produce_a_mapping(self):
        good = binding_request(TRANSACTION)
        cases = [b"", good[:19], good + b"x", b"x" * (MAX_REQUEST + 1),
                 HEADER.pack(0x0101, 0, COOKIE, TRANSACTION), HEADER.pack(0x0001, 0, 0, TRANSACTION),
                 HEADER.pack(0x0001, 4, COOKIE, TRANSACTION),
                 HEADER.pack(0x0001, 4, COOKIE, TRANSACTION) + struct.pack("!HH", 0x8022, 8),
                 HEADER.pack(0x0001, 4, COOKIE, TRANSACTION) + struct.pack("!HH", 0x0006, 0),
                 HEADER.pack(0x0001, 4, COOKIE, TRANSACTION) + struct.pack("!HH", 0x8028, 0)]
        for packet in cases:
            with self.subTest(packet=packet), self.assertRaises(ValueError):
                binding_response(packet, (WAN, 41000))

    def test_software_and_valid_fingerprint_interoperate(self):
        software = struct.pack("!HH", 0x8022, 7) + b"Meshway" + b"\0"
        prefix = HEADER.pack(0x0001, len(software) + 8, COOKIE, TRANSACTION) + software
        fingerprint = struct.pack("!HHI", 0x8028, 4, zlib.crc32(prefix) ^ 0x5354554E)
        request = prefix + fingerprint
        self.assertEqual(mapped_response(binding_response(request, (WAN, 41000)), TRANSACTION), f"{WAN}:41000")
        with self.assertRaises(ValueError):
            binding_response(request[:-1] + bytes([request[-1] ^ 1]), (WAN, 41000))

    def test_optional_attributes_accepted_required_attributes_rejected(self):
        request = HEADER.pack(0x0001, 4, COOKIE, TRANSACTION) + struct.pack("!HH", 0xA123, 0)
        self.assertEqual(mapped_response(binding_response(request, (WAN, 41000)), TRANSACTION), f"{WAN}:41000")
        wrong_transaction = bytes(reversed(TRANSACTION))
        with self.assertRaises(ValueError):
            mapped_response(binding_response(request, (WAN, 41000)), wrong_transaction)


class ObserverTests(unittest.TestCase):
    def probe(self, version):
        family = socket.AF_INET if version == 4 else socket.AF_INET6
        with loopback_observer(version) as (server, _snapshot), socket.socket(family, socket.SOCK_DGRAM) as client:
            client.bind((server[0], 0))
            client.settimeout(1)
            client.sendto(binding_request(TRANSACTION), server)
            payload, source = client.recvfrom(512)
            self.assertEqual(source[:2], server)
            self.assertEqual(mapped_response(payload, TRANSACTION), endpoint(*client.getsockname()[:2]))

    def test_ipv4_real_socket_peer(self):
        self.probe(4)

    def test_ipv6_real_socket_peer(self):
        try:
            with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as listener:
                listener.bind(("::1", 0))
        except OSError:
            self.skipTest("host has no IPv6 loopback")
        self.probe(6)

    def test_source_guard_rejects_non_router_even_inside_subnet(self):
        with loopback_observer(4, allowed="127.0.0.2") as (server, _snapshot), socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            client.bind(("127.0.0.1", 0))
            client.settimeout(0.15)
            client.sendto(binding_request(TRANSACTION), server)
            with self.assertRaises(socket.timeout):
                client.recvfrom(512)

    def test_malformed_datagram_dropped_then_real_binding_works(self):
        with loopback_observer() as (server, _snapshot), socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            client.bind((server[0], 0))
            client.settimeout(0.15)
            client.sendto(b"not-stun", server)
            with self.assertRaises(socket.timeout):
                client.recvfrom(512)
            client.sendto(binding_request(TRANSACTION), server)
            response = client.recvfrom(512)[0]
            self.assertEqual(mapped_response(response, TRANSACTION), endpoint(*client.getsockname()))

    def test_all_packets_count_toward_response_rate_limit(self):
        with loopback_observer() as (server, _snapshot), socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            client.bind((server[0], 0))
            client.settimeout(0.15)
            for _ in range(RATE_PER_SECOND):
                client.sendto(b"malformed", server)
            client.sendto(binding_request(TRANSACTION), server)
            with self.assertRaises(socket.timeout):
                client.recvfrom(512)


class CanaryTests(unittest.TestCase):
    def test_held_socket_has_A_controls_on_both_sides_of_receive_window(self):
        with loopback_observer() as (server, _snapshot), tempfile.TemporaryDirectory(dir=PARENT) as directory:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sender:
                client.bind((server[0], 0))
                client.settimeout(0.02)
                sender.bind((server[0], 0))
                first = canary.binding(client, server)
                marker = Path(directory) / "sent"
                marker.touch()
                self.assertFalse(canary.receive_canary(client, sender.getsockname(), b"nonce", marker, maximum_wait=1, window=0.06))
                self.assertEqual(canary.binding(client, server), first)
                # Delivery would be detected if the nonce is actually received.
                sender.sendto(b"nonce", client.getsockname())
                self.assertTrue(canary.receive_canary(client, sender.getsockname(), b"nonce", marker, maximum_wait=1, window=0.06))
                sender.sendto(b"wrong-nonce", client.getsockname())
                self.assertFalse(canary.receive_canary(client, sender.getsockname(), b"nonce", marker, maximum_wait=1, window=0.06))
                self.assertEqual(canary.binding(client, server), first)

    def test_missing_send_coordination_is_failure_not_filtered_success(self):
        with tempfile.TemporaryDirectory(dir=PARENT) as directory, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            client.bind(("127.0.0.1", 0))
            client.settimeout(0.01)
            with self.assertRaises(ValueError):
                canary.receive_canary(client, ("127.0.0.1", 3479), b"nonce", Path(directory) / "missing", maximum_wait=0.04)

    def test_late_send_coordination_does_not_shorten_receive_window(self):
        with tempfile.TemporaryDirectory(dir=PARENT) as directory, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            client.bind(("127.0.0.1", 0))
            client.settimeout(0.01)
            marker = Path(directory) / "sent"

            def mark_sent():
                time.sleep(0.04)
                marker.touch()

            thread = threading.Thread(target=mark_sent)
            thread.start()
            try:
                started = time.monotonic()
                self.assertFalse(canary.receive_canary(client, ("127.0.0.1", 3479), b"nonce", marker, maximum_wait=0.10, window=0.12))
                self.assertGreaterEqual(time.monotonic() - started, 0.15)
            finally:
                thread.join()


class ReportTests(unittest.TestCase):
    def test_all_scenario_expectations_crosscheck_independent_peers(self):
        for name, ports in SCENARIOS:
            with self.subTest(name=name):
                report = validated(report_for(ports))
                observed = {"a": [endpoint(WAN, p) for p in (ports[0], ports[2]) if p is not None], "b": [] if ports[1] is None else [endpoint(WAN, ports[1])]}
                crosscheck(report, WAN, ports, observed)

    def test_partial_comparison_cannot_be_false(self):
        report = report_for((41000, None, 41000))
        self.assertIsNone(validated(report)["mapping_varies_by_destination"])
        report["mapping_varies_by_destination"] = False
        with self.assertRaises(CheckError):
            validated(report)

    def test_time_varying_A_invalidates_destination_comparison(self):
        report = report_for((41003, 41004, 41004))
        self.assertIsNone(validated(report)["mapping_varies_by_destination"])
        self.assertIs(validated(report)["mapping_stable_during_check"], False)
        report["mapping_varies_by_destination"] = True
        with self.assertRaises(CheckError):
            validated(report)

    def test_server_spoofed_address_fails_real_peer_crosscheck(self):
        with self.assertRaises(CheckError):
            crosscheck(validated(report_for()), WAN, (41000, 41000, 41000), {"a": ["192.0.2.9:41000"] * 2, "b": [f"{WAN}:41000"]})

    def test_strict_json_rejects_duplicate_keys_logs_and_nonfinite(self):
        for raw in ('{"a":1,"a":2}', '{"rtt":NaN}', '{"rtt":Infinity}', 'log\n{}', '{}\n{}'):
            with self.subTest(raw=raw), self.assertRaises(CheckError):
                strict_json(raw)

    def test_invalid_contract_states_fail(self):
        changes = [
            ("schema_version", True), ("source", "daemon"), ("udp_observed", 1),
            ("udp_observed", False), ("mapping_stable_during_check", False),
            ("address_family", "ipv6"), ("bound_socket", "localhost:1000"),
            ("valid_until", "1970-01-01T00:30:00Z"), ("measured_at", "1970-01-01T00:00:00Z"),
            ("completed_at", "1970-01-01T00:16:40"),
        ]
        for key, value in changes:
            report = report_for()
            report[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(CheckError):
                validated(report)
        for key, value in (("rtt_ms", True), ("elapsed_ms", -1), ("mapped_endpoint", "outside.example:41000")):
            report = report_for()
            report["samples"][0][key] = value
            with self.subTest(key=key), self.assertRaises(CheckError):
                validated(report)
        with self.assertRaises(CheckError):
            validate_report(json.dumps(report_for()), 1, SERVERS, 999, 1004)
        with self.assertRaises(CheckError):
            validate_report(json.dumps(report_for()), 0, [SERVERS[0], SERVERS[0]], 999, 1004)


class NamespaceTests(unittest.TestCase):
    def test_shared_flock_protocol_rejects_concurrent_acquisition_and_releases(self):
        # A separate private test file verifies the protocol without contending
        # with Main's actual Docker VM lock.
        with tempfile.TemporaryDirectory(dir=PARENT) as directory:
            path = Path(directory) / "test.lock"
            with patch.dict(os.environ, MESHWAY_PERF_LOCK=str(path)):
                with run.performance_lock():
                    with path.open("r+") as other:
                        with self.assertRaises(BlockingIOError):
                            fcntl.flock(other.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                with path.open("r+") as other:
                    fcntl.flock(other.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                os.chmod(path, 0o644)
                with self.assertRaises(RuntimeError):
                    with run.performance_lock():
                        self.fail("non-private lock accepted")

    def test_container_plans_have_no_host_network_mounts_or_published_ports(self):
        for role in ("client", "router", "a", "b"):
            args = run.container_args("test", role, "unique", "internal", "sha256:" + "a" * 64)
            self.assertNotIn("--privileged", args)
            self.assertNotIn("--mount", args)
            self.assertNotIn("--volume", args)
            self.assertNotIn("--publish", args)
            self.assertEqual(args[args.index("--network") + 1], "internal")
            self.assertEqual(args[args.index("--cap-drop") + 1], "ALL")
            self.assertEqual("--cap-add" in args, role in ("client", "router"))
            self.assertIn("mode=0700", args[args.index("--tmpfs") + 1])
            self.assertIn("--read-only", args)
            self.assertEqual(args[-2], "sha256:" + "a" * 64)
            self.assertEqual(args[args.index("--pids-limit") + 1], "64")

    def test_kernel_commands_only_reset_namespace_and_target_fixture_IPs(self):
        for name, ports in SCENARIOS:
            commands = router_commands(name, "198.51.100.2", WAN, "192.0.2.10", "192.0.2.11", ports)
            self.assertIn(["conntrack", "-F"], commands)
            self.assertTrue(all(command[0] in ("iptables", "conntrack") for command in commands))
            snat = [command for command in commands if "SNAT" in command]
            self.assertEqual(len(snat), int(ports[0] is not None) + int(ports[1] is not None))
            for command in snat:
                self.assertEqual(command[command.index("-s") + 1], "198.51.100.2")
                self.assertIn(command[command.index("-d") + 1], ("192.0.2.10", "192.0.2.11"))

    def test_allowlisted_counters_exclude_raw_rule_output(self):
        value = run.counters('[2:80] -A POSTROUTING -s 198.51.100.2/32 -m comment --comment "netcheck-snat-a" -j SNAT\n[5:200] -A INPUT -m comment --comment ignored -j DROP')
        self.assertEqual(value, {"netcheck-snat-a": {"packets": 2, "bytes": 80}})
        with self.assertRaises(CheckError):
            run.counters('[1:20] -A INPUT -m comment --comment "netcheck-unknown" -j DROP')

    def test_conntrack_checks_single_socket_and_real_reverse_snat_tuple(self):
        rows = "\n".join(
            f"udp 17 29 src=198.51.100.2 dst={server} sport=53000 dport=3478 src={server} dst={WAN} sport=3478 dport=41000 [ASSURED] mark=0 use=1"
            for server in ("192.0.2.10", "192.0.2.11")
        )
        report = report_for()
        args = ("198.51.100.2", WAN, ["192.0.2.10", "192.0.2.11"], 53000, report["samples"])
        self.assertEqual(len(run.conntrack_mappings(rows, *args)), 2)
        with self.assertRaises(CheckError):
            run.conntrack_mappings(rows.replace("sport=53000", "sport=53001", 1), *args)
        with self.assertRaises(CheckError):
            run.conntrack_mappings(rows.replace("dport=41000", "dport=41001", 1), *args)

    def test_cleanup_inspects_ownership_and_continues_after_failure(self):
        class FakeDocker:
            def __init__(self):
                self.removed = []

            def inspect(self, kind, target):
                role = {"a": "a", "b": "b", "lan": "lan"}[target]
                labels = {run.LABEL: "foreign" if role == "a" else "our-run", run.ROLE_LABEL: role}
                return {"Id": ("1" if role == "a" else "2" if role == "b" else "3") * 64,
                        "Config": {"Labels": labels}, "Labels": labels}

            def text(self, args):
                self.removed.append(args)

        docker = FakeDocker()
        harness = run.Harness(docker, "our-run")
        harness.resources = [{"kind": "network", "role": "lan", "name": "lan", "id": None},
                             {"kind": "container", "role": "b", "name": "b", "id": None},
                             {"kind": "container", "role": "a", "name": "a", "id": None}]
        cleanup = harness.cleanup()
        self.assertFalse(cleanup["complete"])
        self.assertEqual(docker.removed, [["container", "rm", "--force", "2" * 64], ["network", "rm", "3" * 64]])
        self.assertEqual(cleanup["resources"][0]["status"], "failed_inspect_or_remove")

    def test_cleanup_finds_interrupted_creation_by_exact_name(self):
        class FakeDocker:
            def inspect(self, kind, name):
                self.inspected = name
                return {"Id": "a" * 64, "Config": {"Labels": {run.LABEL: "our-run", run.ROLE_LABEL: "client"}}}

            def text(self, args):
                self.removed = args

        docker = FakeDocker()
        harness = run.Harness(docker, "our-run")
        harness.resources = [{"kind": "container", "role": "client", "name": "exact-attempted-name", "id": None}]
        self.assertTrue(harness.cleanup()["complete"])
        self.assertEqual(docker.inspected, "exact-attempted-name")
        self.assertEqual(docker.removed, ["container", "rm", "--force", "a" * 64])

    def test_dedicated_image_and_native_architecture_gate(self):
        class FakeDocker:
            def json(self, args):
                if args[0] == "image":
                    return [{"Id": "sha256:" + "a" * 64, "Architecture": "arm64", "Os": "linux", "Config": {}}]
                return {"os": "linux", "architecture": "aarch64"}

        harness = run.Harness(FakeDocker(), "test")
        with patch("run.platform.machine", return_value="arm64"):
            self.assertEqual(harness.image("meshway-netcheck:test")["immutable_id"], "sha256:" + "a" * 64)
            with self.assertRaises(CheckError):
                harness.image("meshway-live:latest")
        with patch("run.platform.machine", return_value="x86_64"), self.assertRaises(CheckError):
            harness.image("meshway-netcheck:test")

    def test_failure_and_interrupt_always_write_structured_cleanup_result(self):
        class FakeHarness:
            stage = "udp_drop"
            cleanup_complete = True
            failure = CheckError("fixed check failed")

            def __init__(self, _docker, _run_id):
                pass

            def image(self, _name):
                return {"immutable_id": "sha256:" + "a" * 64}

            def provision(self, _identifier):
                return {}

            def scenario(self, _name, _ports):
                raise self.failure

            def cleanup(self):
                return {"complete": self.cleanup_complete, "resources": []}

        for failure, complete, expected_exit, expected_status in (
            (CheckError("fixed check failed"), True, 1, "failed"),
            (run.Interrupted(), True, 130, "interrupted"),
            (run.Interrupted(), False, 1, "failed"),
        ):
            with self.subTest(failure=type(failure).__name__, complete=complete), tempfile.TemporaryDirectory(dir=PARENT) as parent:
                FakeHarness.failure = failure
                FakeHarness.cleanup_complete = complete
                with patch("run.Harness", FakeHarness), patch("run.performance_lock", return_value=nullcontext(123)), \
                        patch("run.signal.signal"), patch("run.sys.argv", ["run.py", "--image", "meshway-netcheck:test", "--artifact-parent", parent]), \
                        patch("run.sys.stdout", new_callable=io.StringIO):
                    self.assertEqual(run.main(), expected_exit)
                artifacts = list(Path(parent).glob("meshway-netcheck-*/result.json"))
                self.assertEqual(len(artifacts), 1)
                result = json.loads(artifacts[0].read_text())
                self.assertEqual(result["status"], expected_status)
                self.assertEqual(result["cleanup"]["complete"], complete)
                self.assertEqual(result["failure_stage"], "udp_drop")
                self.assertEqual(artifacts[0].stat().st_mode & 0o077, 0)


if __name__ == "__main__":
    unittest.main()
