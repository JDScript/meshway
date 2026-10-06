"""Report regression checks; never launch Docker or measure performance."""
import unittest
import json
from pathlib import Path
import tempfile

from perf.dataplane import last_profile, receiver_metrics, summary


class DataplaneReportTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
