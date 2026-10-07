"""Report regression checks; never launch Docker or measure performance."""
import unittest
import json
from pathlib import Path
import tempfile
from unittest.mock import patch
from types import SimpleNamespace

from perf.dataplane import MODES, configurations, image_for, last_profile, namespace_options, network_counters, network_delta, parse_args, profile_delta, receiver_metrics, summary, thread_delta, wireguard_transfers


class DataplaneReportTests(unittest.TestCase):
    def test_quick_is_a_single_configuration_preset_and_report_is_diagnostic(self):
        for extra, mode in (([], "manager-relay"), (["--modes", "secure-relay"], "secure-relay")):
            args = parse_args(["--quick", *extra])
            self.assertEqual(configurations(args), [(mode, 32, 0)])
            self.assertEqual((args.repeats, args.seconds), (1, 4))
            self.assertEqual(len(configurations(args)) * args.repeats * len(args.traffic), 2)
            report = summary(dict(environment={}, results=[], options=vars(args)))
            self.assertIn("diagnostic quick run，不作最终矩阵", report.splitlines()[2])
        args = parse_args(["--quick", "--profile", "128", "--batch", "1", "--traffic", "tcp"])
        self.assertEqual(configurations(args), [("manager-relay", 1, 128)])
        with self.assertRaises(SystemExit), patch("sys.stderr"):
            parse_args(["--quick", "--modes", "secure-relay", "manager-relay"])

    def test_normal_defaults_preserve_the_full_matrix(self):
        args = parse_args([])
        self.assertEqual(args.modes, list(MODES))
        self.assertEqual((args.batch, args.profile, args.repeats, args.seconds), ([1, 32], [0, 128], 2, 8))
        self.assertEqual(len(configurations(args)), 27)
        with self.assertRaises(SystemExit), patch("sys.stderr"):
            parse_args(["--repeats", "1"])

    def test_kernel_counters_keep_udp_and_tun_loss_distinct(self):
        snmp = "Udp: InDatagrams InErrors RcvbufErrors SndbufErrors\nUdp: 900 17 16 0"
        dev = "header\nheader\n tun0: 100 2 0 0 0 0 0 0 200 3 0 7 0 0 0 0"
        with patch("perf.dataplane.command", side_effect=[snmp, dev, "socket memory", "socket drops"]):
            before = network_counters("owned")
        after = json.loads(json.dumps(before))
        after["protocols"]["Udp"]["RcvbufErrors"] += 5
        after["interfaces"]["tun0"]["txDropped"] += 9
        delta = network_delta(before, after)
        self.assertEqual(delta["protocols"]["Udp"]["RcvbufErrors"], 5)
        self.assertEqual(delta["interfaces"]["tun0"]["txDropped"], 9)
        self.assertEqual(delta["interfaces"]["tun0"]["rxDropped"], 0)
        self.assertEqual(before["sockets"], "socket memory")
        self.assertEqual(before["udpTable"], "socket drops")

    def test_misaligned_kernel_counter_headers_are_rejected(self):
        with patch("perf.dataplane.command", side_effect=["Udp: InErrors\nTcp: 1", "header\nheader"]):
            with self.assertRaisesRegex(ValueError, "layout"):
                network_counters("owned")

    def test_untagged_complete_image_is_reused_but_partial_layers_are_not(self):
        complete = dict(Id="complete", Os="linux", Architecture="arm64",
                        Config=dict(Entrypoint=["/usr/local/lib/meshway-lab/entrypoint.sh"]))
        partial = dict(complete, Id="partial", Config={})
        with tempfile.TemporaryDirectory() as directory, patch("perf.dataplane.image_builder") as builder, patch("perf.dataplane.command") as command:
            builder.return_value.source_digest.return_value = "locked"
            command.side_effect = ["partial\ncomplete", json.dumps([partial]), json.dumps([complete])]
            self.assertEqual(image_for("linux/arm64", Path(directory))["imageId"], "complete")
            self.assertIn("--all", command.call_args_list[0].args)
            builder.return_value.build.assert_not_called()

    def test_reused_process_profile_deltas_do_not_replay_previous_sample(self):
        def metric(calls, ns):
            return dict(name="same", calls_started=calls, times=dict(self_cpu_ns=ns, self_cpu_samples=calls, max_wall_ns=30, cpu_ns=None))
        result = profile_delta([metric(8, 100), metric(3, 30)], [metric(4, 40), metric(2, 20)])
        self.assertEqual(result, [dict(name="same", calls_started=5, times=dict(self_cpu_ns=70, self_cpu_samples=5, max_wall_ns=30, cpu_ns=None))])
        with self.assertRaisesRegex(ValueError, "backwards"):
            profile_delta([metric(1, 1)], [metric(2, 2)])

    def test_thread_cpu_fraction_uses_its_own_snapshot_wall_interval(self):
        before = dict(at=10, threads={"12": dict(name="owner", cpuSeconds=3)})
        after = dict(at=20, threads={"12": dict(name="owner", cpuSeconds=8)})
        self.assertEqual(thread_delta(before, after)[0]["cpuActivePercent"], 50)

    def test_mode_filter_preserves_kernel_reference_and_profile_matrix(self):
        args = SimpleNamespace(modes=["secure-relay", "raw", "plain-batch"], batch=[1, 32], profile=[0, 128])
        configs = configurations(args)
        self.assertEqual(len(configs), 9)
        self.assertEqual([c for c in configs if c[0] == "raw"], [("raw", 0, 0)])
        self.assertEqual({c[0] for c in configs}, set(args.modes))
        args.modes = MODES
        self.assertEqual(len(configurations(args)), 27)

    def test_tcp_receiver_uses_server_role_not_client_sender_flag(self):
        received = dict(start=0, end=7.998435, seconds=7.998435, bytes=2219048960,
                        bits_per_second=2219483146.390513, sender=False)
        data = {"end": {"sum_received": dict(received, sender=True)},
                "server_output_json": {"end": {"sum_received": received}}}
        self.assertEqual(receiver_metrics(data, "tcp", 1200), received)

    def test_udp_uses_real_received_bytes_not_iperf316_zero_sum(self):
        received = dict(sender=False, seconds=8, packets=100, lost_packets=25,
                        bytes=90000, bits_per_second=90000, lost_percent=25)
        data = {"server_output_json": {"end": {"sum": {"bytes": 0, "bits_per_second": 0},
                                                "sum_received": received}}}
        self.assertEqual(receiver_metrics(data, "udp", 1200), received)
        received["bytes"] = 0
        with self.assertRaisesRegex(ValueError, "disagree"):
            receiver_metrics(data, "udp", 1200)

    def test_cumulative_profiles_are_not_double_counted(self):
        with tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parent) as directory:
            path = Path(directory) / "stderr"
            path.write_text("\n".join(json.dumps({"fields": {"metrics_json": json.dumps(value)}})
                                      for value in ([{"calls": 10}], [{"calls": 20}])) + "\n")
            self.assertEqual(last_profile(path), [{"calls": 20}])

    def test_failed_delivery_is_not_a_zero_or_successful_repeat(self):
        common = dict(mode="plain", batch=32, profile=0, traffic="udp")
        report = summary({"environment": {}, "results": [
            dict(common, status="ok", goodputMbps=100, packetsPerSecond=1000,
                 lostPercent=2, cpuPercent=50),
            dict(common, status="failed", goodputMbps=None, packetsPerSecond=None,
                 lostPercent=None, cpuPercent=None),
        ]})
        self.assertIn("| plain | 32 | 0 | udp | 1 | 100.00 | 1000.00 | 2.00 | 50.00 |", report)
        self.assertIn("Failed samples: 1", report)

    def test_profile_modes_and_tcp_unknown_pps_remain_separate(self):
        common = dict(mode="plain", batch=32, traffic="tcp", status="ok",
                      packetsPerSecond=None, lostPercent=None, cpuPercent=80)
        report = summary({"environment": {}, "results": [
            dict(common, profile=0, goodputMbps=120),
            dict(common, profile=0, goodputMbps=100),
            dict(common, profile=128, goodputMbps=40),
        ]})
        self.assertIn("| plain | 32 | 0 | tcp | 2 | 110.00 | — | — | 80.00 |", report)
        self.assertIn("| plain | 32 | 128 | tcp | 1 | 40.00 | — | — | 80.00 |", report)

    def test_secure_delta_matches_batch_profile_and_excludes_failed(self):
        common = dict(batch=32, profile=0, traffic="tcp", status="ok",
                      packetsPerSecond=None, lostPercent=None, cpuPercent=80)
        report = summary({"environment": {}, "results": [
            dict(common, mode="plain", goodputMbps=100),
            dict(common, mode="secure-direct", goodputMbps=60),
            dict(common, mode="secure-relay", goodputMbps=40),
            dict(common, mode="secure-relay", goodputMbps=0, status="failed"),
            dict(common, mode="plain", goodputMbps=1000, profile=128),
        ]})
        self.assertIn("| secure-direct | 32 | 0 | tcp | -40.00 | -40.00 | +0.00 | — |", report)
        self.assertIn("| secure-relay | 32 | 0 | tcp | -60.00 | -60.00 | +0.00 | — |", report)

    def test_real_profile_schema_counts_started_calls(self):
        def metric(name, calls):
            return dict(name=name, calls_started=calls,
                        times=dict(self_cpu_ns=100, self_cpu_samples=1))
        record = dict(mode="secure-relay", batch=32, profile=128, traffic="udp",
                      status="ok", directory="secure-relay-sample", goodputMbps=40,
                      packetsPerSecond=4000, lostPercent=10, cpuPercent=100,
                      profiles={"a": [metric("control.guard", 200), metric("l3.packet.origin", 2)],
                                "r": [metric("control.guard", 50)],
                                "b": [metric("control.guard", 100)]})
        report = summary({"environment": {}, "results": [record]})
        self.assertIn("| secure-relay-sample | 350 | 2 | 175.00 |", report)

    def test_wireguard_delta_pairs_topology_and_traffic_not_meshway_batch(self):
        common = dict(batch=0, profile=0, traffic="udp", status="ok", goodputMbps=800,
                      packetsPerSecond=80000, lostPercent=4, cpuPercent=50)
        report = summary({"environment": {}, "results": [
            dict(common, mode="wg-direct", goodputMbps=1000),
            dict(common, mode="wg-direct", goodputMbps=600),
            dict(common, mode="wg-direct", goodputMbps=0, status="failed"),
            dict(common, mode="wg-relay", goodputMbps=400, packetsPerSecond=40000),
            dict(common, mode="wg-direct", traffic="tcp", goodputMbps=9000,
                 packetsPerSecond=None, lostPercent=None),
            dict(common, mode="secure-direct", batch=32, goodputMbps=500,
                 packetsPerSecond=50000, lostPercent=20, cpuPercent=250),
            dict(common, mode="secure-relay", batch=1, profile=128, goodputMbps=200,
                 packetsPerSecond=20000, lostPercent=50, cpuPercent=300),
        ]})
        self.assertIn("| wg-direct | 0 | 0 | udp | 2 | 800.00 | 80000.00 | 4.00 | 50.00 |", report)
        self.assertIn("| secure-direct | wg-direct | 32 | 0 | udp | -300.00 | -30000.00 | +16.00 | +200.00 |", report)
        self.assertIn("| secure-relay | wg-relay | 1 | 128 | udp | -200.00 | -20000.00 | +46.00 | +250.00 |", report)
        self.assertIn("total kernel CPU is unmeasured, not zero", report)

    def test_missing_wireguard_and_tcp_pps_are_not_zero_deltas(self):
        common = dict(batch=32, profile=0, traffic="tcp", status="ok", goodputMbps=100,
                      packetsPerSecond=None, lostPercent=None, cpuPercent=50)
        report = summary({"environment": {}, "results": [
            dict(common, mode="secure-direct"), dict(common, mode="secure-relay"),
            dict(common, mode="wg-direct", batch=0, goodputMbps=400),
            dict(common, mode="wg-relay", batch=0, status="failed"),
        ]})
        self.assertIn("| secure-direct | wg-direct | 32 | 0 | tcp | -300.00 | — | — | +0.00 |", report)
        self.assertIn("| secure-relay | wg-relay | 32 | 0 | tcp | — | — | — | — |", report)

    def test_wireguard_minimal_namespace_privileges(self):
        for role in ("a", "b", "r"):
            options = namespace_options("wg-relay", role)
            self.assertIn("NET_ADMIN", options)
            self.assertNotIn("--device", options)
            self.assertEqual("net.ipv4.ip_forward=1" in options, role == "r")
        self.assertEqual(namespace_options("secure-relay", "r"), [])

    def test_wireguard_requires_each_hop_bidirectional_authenticated_traffic(self):
        evidence = {"a": {"wg0": {}}, "r": {"wg0": {}, "wg1": {}}, "b": {"wg0": {}}}
        with patch("perf.dataplane.command", side_effect=["123 500 600", "123 600 500", "0 0 0"]):
            with self.assertRaisesRegex(RuntimeError, "handshake or bidirectional"):
                wireguard_transfers({role: role for role in evidence}, evidence)
        with patch("perf.dataplane.command", return_value="123 500 600"):
            wireguard_transfers({role: role for role in evidence}, evidence)
        self.assertEqual(evidence["r"]["wg1"], dict(handshakeUnixSeconds=123, receivedBytes=500, sentBytes=600))


if __name__ == "__main__":
    unittest.main()
