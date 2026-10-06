"""Report regression checks; never launch Docker or measure performance."""
import unittest

from perf.dataplane import summary


class DataplaneReportTests(unittest.TestCase):
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
