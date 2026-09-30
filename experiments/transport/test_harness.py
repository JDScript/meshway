"""Safety-boundary checks; real topology/path checks run in benchmark.py."""

import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import benchmark


class HarnessSafetyTests(unittest.TestCase):
    def fixture(self):
        args = benchmark.arguments(["--output", "/unused/result.json", "--image", "caller-owned:lab"])
        return benchmark.Fixture(args, {"run_id": "meshway-lab-test"})

    def test_cleanup_refuses_name_collision_without_ownership(self):
        fixture = self.fixture()
        fixture.containers_owned.append("colliding-container-name")
        inspected = subprocess.CompletedProcess([], 0, json.dumps([{"Config": {"Labels": {}}}]), "")
        with patch.object(benchmark, "docker", return_value=inspected) as docker:
            errors = fixture.cleanup()
        self.assertEqual(len(errors), 1)
        self.assertIn("different labels", errors[0])
        self.assertEqual([call.args[:2] for call in docker.call_args_list], [("container", "inspect")])

    def test_cleanup_is_exact_and_retains_caller_image(self):
        fixture = self.fixture()
        fixture.containers_owned.extend(["owned-nyc", "owned-sha"])
        fixture.networks_owned.append("owned-bridge")
        labels = {benchmark.LABEL: "transport", benchmark.RUN_LABEL: fixture.prefix}

        def docker_result(kind, command, *args, **kwargs):
            if command == "inspect":
                info = {"Config": {"Labels": labels}} if kind == "container" else {"Labels": labels}
                return subprocess.CompletedProcess([], 0, json.dumps([info]), "")
            return subprocess.CompletedProcess([], 0, "", "")

        with patch.object(benchmark, "docker", side_effect=docker_result) as docker:
            self.assertEqual(fixture.cleanup(), [])
        removals = [call.args for call in docker.call_args_list if call.args[1] == "rm"]
        self.assertEqual(removals, [
            ("container", "rm", "--force", "owned-sha"),
            ("container", "rm", "--force", "owned-nyc"),
            ("network", "rm", "owned-bridge"),
        ])
        self.assertFalse(any(call.args[0] == "image" for call in docker.call_args_list))

    def test_missing_daemon_cleanup_is_reported(self):
        fixture = self.fixture()
        fixture.networks_owned.append("owned-bridge")
        unavailable = subprocess.CompletedProcess([], 1, "", "Cannot connect to Docker daemon")
        with patch.object(benchmark, "docker", return_value=unavailable):
            self.assertIn("Cannot connect", fixture.cleanup()[0])

    def test_unlimited_or_nonfinite_load_rejected(self):
        for value in ("0", "0M", "2G", "nan", "inf", "-1", "1M/2"):
            with self.subTest(rate=value), self.assertRaises(benchmark.argparse.ArgumentTypeError):
                benchmark.bounded_rate(value)
        for value in ("nan", "inf", "0", "9"):
            with self.subTest(cpus=value), self.assertRaises(benchmark.argparse.ArgumentTypeError):
                benchmark.positive_cpus(value)
        self.assertEqual(benchmark.bounded_rate("20M"), "20M")

    def test_foreign_image_architecture_rejected_before_container_creation(self):
        fixture = self.fixture()
        foreign = [{"Os": "linux", "Architecture": "amd64"}]
        with patch.object(benchmark, "docker_json", return_value=foreign), self.assertRaisesRegex(benchmark.LabError, "Refusing emulated"):
            fixture.prepare_image("arm64")
        self.assertEqual(fixture.containers_owned, [])

    def test_udp_summary_uses_receiver_loss_and_keeps_process_cpu_scope(self):
        raw = {"end": {
            "sum_sent": {"bits_per_second": 20_000_000},
            "sum_received": {"bits_per_second": 19_000_000, "lost_packets": 5, "packets": 100, "lost_percent": 5, "jitter_ms": 0.12},
            "cpu_utilization_percent": {"host_total": 2, "remote_total": 3},
        }}
        summary = benchmark.summarize(raw)
        self.assertEqual(summary["udp"]["lost_percent"], 5)
        self.assertEqual(summary["receiver_bits_per_second"], 19_000_000)
        self.assertEqual(summary["iperf_process_cpu_percent"]["host_total"], 2)

    def test_caller_image_entrypoint_is_overridden(self):
        fixture = self.fixture()
        fixture.networks["a"] = {"name": "owned-network"}
        with patch.object(benchmark, "docker") as docker, patch.object(fixture, "exec"), \
                patch.object(fixture, "copy_file"), patch.object(fixture, "probe", return_value=True):
            fixture.create_node("NYC", "a", "direct")
        command = docker.call_args_list[0].args
        self.assertEqual(command[command.index("--entrypoint") + 1], "python3")
        self.assertIn("caller-owned:lab", command)
        self.assertNotIn("--privileged", command)

    def test_network_mtu_is_explicit_and_subnet_is_docker_allocated(self):
        fixture = self.fixture()
        fixture.args.mtu = 1280
        inspected = [{"Id": "network-id", "Internal": True, "IPAM": {"Config": [{"Subnet": "192.0.2.0/24"}]}}]
        with patch.object(benchmark, "docker") as docker, patch.object(benchmark, "docker_json", return_value=inspected):
            fixture.create_network("a", "routed")
        self.assertIn("com.docker.network.driver.mtu=1280", docker.call_args.args)
        self.assertNotIn("--subnet", docker.call_args.args)
        self.assertEqual(fixture.networks["a"]["requested_mtu"], 1280)

    def test_private_transfer_never_puts_content_in_argv_or_error(self):
        fixture = self.fixture()
        fixture.nodes["NYC"] = "owned-nyc"
        with tempfile.TemporaryDirectory(prefix="meshway-harness-test-") as scratch:
            source = Path(scratch) / "private.json"
            source.write_text("TEST-SECRET")

            def failed_copy(command, **kwargs):
                self.assertNotIn("TEST-SECRET", " ".join(command))
                self.assertEqual(kwargs["stdin"].read(), b"TEST-SECRET")
                return subprocess.CompletedProcess(command, 1, b"TEST-SECRET", b"TEST-SECRET")

            with patch.object(benchmark.subprocess, "run", side_effect=failed_copy), self.assertRaises(benchmark.LabError) as error:
                fixture.copy_file("NYC", source, "/run/private.json")
            self.assertNotIn("TEST-SECRET", str(error.exception))


if __name__ == "__main__":
    unittest.main()
