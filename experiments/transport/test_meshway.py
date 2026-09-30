"""Credential isolation, reverse-channel, and measurement-boundary checks."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import meshway
import meshway_node
import node


class MeshwaySafetyTests(unittest.TestCase):
    def fixture(self, directory=None):
        args = meshway.arguments(["--output", "/unused/result.json"])
        args.temp_dir = directory
        fixture = meshway.MeshwayFixture(args, {"run_id": "meshway-lab-test"})
        fixture.nodes = {name: f"owned-{name.lower()}" for name in ("NYC", "LAX", "SHA")}
        return fixture

    def test_status_never_copies_credentials_or_arbitrary_strings(self):
        status = meshway.public_status({
            "node_id": "public-nyc", "experiment": True, "authenticated_peers": ["public-lax"],
            "tun_enabled": True, "private_key": "SECRET", "vless_uuid": "SECRET",
            "statistics": {"sent": 10, "password": "SECRET", "private_key": 12345,
                           "nested": {"received": 11, "secret": "SECRET"}},
            "tun": {"received": 12}, "vless_handshake_failures": 2,
        })
        self.assertNotIn("SECRET", json.dumps(status))
        self.assertNotIn("private_key", status)
        self.assertEqual(status["statistics"]["sent"], 10)
        self.assertNotIn("private_key", status["statistics"])
        self.assertEqual(status["statistics"]["nested"], {"received": 11})

    def test_empty_meshway_image_cannot_trigger_automatic_build(self):
        fixture = self.fixture()
        fixture.image = ""
        with patch.object(meshway.Fixture, "prepare_image") as inherited, self.assertRaisesRegex(meshway.LabError, "never builds"):
            fixture.prepare_image("arm64")
        inherited.assert_not_called()

    def test_authenticated_graph_requires_reverse_channel_identity(self):
        statuses = {
            "NYC": {"node_id": "nyc", "authenticated_peers": ["lax"], "tun_enabled": True},
            "LAX": {"node_id": "lax", "authenticated_peers": ["nyc", "sha"], "tun_enabled": False},
            "SHA": {"node_id": "sha", "authenticated_peers": ["lax"], "tun_enabled": True},
        }
        self.assertTrue(meshway.authenticated_graph(statuses))
        statuses["LAX"]["authenticated_peers"] = ["nyc"]
        self.assertFalse(meshway.authenticated_graph(statuses))

    def test_generator_copies_only_each_nodes_own_private_config(self):
        with tempfile.TemporaryDirectory(prefix="meshway-harness-test-") as scratch:
            fixture = self.fixture(Path(scratch))
            copies = []

            def docker_result(*args, **kwargs):
                if args[0] == "run":
                    mount = args[args.index("--mount") + 1]
                    directory = Path(mount.split("source=", 1)[1].split(",target=", 1)[0])
                    self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
                    self.assertEqual(args[args.index("--user") + 1], f"{os.getuid()}:{os.getgid()}")
                    self.assertEqual(args[args.index("--network") + 1], "none")
                    for name in ("nyc", "lax", "sha"):
                        config = directory / f"{name}.json"
                        config.write_text(json.dumps({"private_key": f"SECRET-{name}"}))
                        config.chmod(0o600)
                return subprocess.CompletedProcess([], 0, "", "")

            with patch.object(meshway, "docker", side_effect=docker_result), \
                    patch.object(fixture, "address", return_value="192.0.2.2"), \
                    patch.object(fixture, "copy_file", side_effect=lambda node, source, destination: copies.append((source.name, node, destination))):
                fixture.generate_configs("vless")
            self.assertEqual(copies, [(f"{name.lower()}.json", name, "/run/meshway-lab/config.json") for name in ("NYC", "LAX", "SHA")])
            self.assertEqual(list(Path(scratch).iterdir()), [])

    def test_unsafe_generated_permissions_are_rejected_before_copy(self):
        with tempfile.TemporaryDirectory(prefix="meshway-harness-test-") as scratch:
            fixture = self.fixture(Path(scratch))

            def docker_result(*args, **kwargs):
                if args[0] == "run":
                    mount = args[args.index("--mount") + 1]
                    directory = Path(mount.split("source=", 1)[1].split(",target=", 1)[0])
                    config = directory / "nyc.json"
                    config.write_text("{}")
                    config.chmod(0o644)
                return subprocess.CompletedProcess([], 0, "", "")

            try:
                with patch.object(meshway, "docker", side_effect=docker_result), \
                        patch.object(fixture, "address", return_value="192.0.2.2"), \
                        patch.object(fixture, "copy_file") as copy_file, \
                        self.assertRaisesRegex(meshway.LabError, "0600"):
                    fixture.generate_configs("udp")
                copy_file.assert_not_called()
            finally:
                if fixture.secret_directory:
                    fixture.secret_directory.cleanup()

    def test_vless_direction_is_sha_dialing_lax(self):
        sha, lax = "192.0.2.3", "192.0.2.2"
        established = {"state": "01", "local": {"address": sha, "port": 54321}, "remote": {"address": lax, "port": 41001}}
        meshway.assert_sha_dials_lax([established], sha, lax)
        with self.assertRaises(meshway.LabError):
            meshway.assert_sha_dials_lax([{**established, "local": established["remote"], "remote": established["local"]}], sha, lax)
        listener = {"state": "0A", "local": {"address": sha, "port": 41001}, "remote": {"address": "0.0.0.0", "port": 0}}
        with self.assertRaises(meshway.LabError):
            meshway.assert_sha_dials_lax([established, listener], sha, lax)

    def test_cpu_window_uses_real_sample_interval(self):
        before = {"LAX": {"monotonic_ns": 1_000_000_000, "files": {"cpu.stat": "usage_usec 200000\nnr_throttled 2\n"}}}
        after = {"LAX": {"monotonic_ns": 3_000_000_000, "files": {"cpu.stat": "usage_usec 700000\nnr_throttled 5\n"}}}
        delta = meshway.cpu_window(before, after)["nodes"]["LAX"]
        self.assertEqual(delta["cpu_percent_one_core"], 25)
        self.assertEqual(delta["nr_throttled_delta"], 3)

    def test_comparison_marks_different_duration_and_payload_mtu(self):
        case = {"id": "tcp-p1-forward-1", "protocol": "tcp", "summary": {"receiver_bits_per_second": 50}}
        settings = {"seconds": 3, "cpus": 1, "memory": "256m", "mtu": 1500}
        baseline = {"settings": settings, "routed": {"cases": [{**case, "status": "ok", "summary": {"receiver_bits_per_second": 100}}]}}
        comparison = meshway.compare_case(case, baseline, {**settings, "seconds": 1})
        self.assertEqual(comparison["meshway_over_baseline_throughput_ratio"], 0.5)
        self.assertIn("seconds", comparison["measurement_setting_differences"])
        self.assertIn("payload_path_mtu", comparison["measurement_setting_differences"])

    def test_supervisor_does_not_signal_reused_pid(self):
        with tempfile.TemporaryDirectory(prefix="meshway-harness-test-") as scratch:
            root = Path(scratch)
            (root / "agent-pid.json").write_text(json.dumps({"pid": 42, "start_ticks": 100}))
            with patch.object(meshway_node, "ROOT", root), \
                    patch.object(meshway_node, "process_identity", return_value={"state": "S", "start_ticks": 200}):
                self.assertFalse(meshway_node.agent_state()["running"])

    def test_debian_node_info_uses_dpkg_not_apk(self):
        with patch.object(node.shutil, "which", return_value=None), \
                patch.object(node, "cgroup_info", return_value={"files": {}}), \
                patch.object(node.Path, "read_text", return_value="cpu"), \
                patch.object(node.os, "sched_getaffinity", return_value={0}, create=True), \
                patch.object(node.subprocess, "check_output", return_value="installed") as run:
            node.info()
        self.assertEqual(run.call_args_list[-1].args[0][0], "dpkg-query")


if __name__ == "__main__":
    unittest.main()
