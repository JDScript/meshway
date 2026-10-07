import unittest

import crypto


class CryptoReportTests(unittest.TestCase):
    def test_quantiles_preserve_operation_and_length_boundaries(self):
        samples = [dict(operation=operation, bytes=size, iterations=100, cpuNs=cpu, wallNs=cpu * 2)
                   for operation, size, cpu in [("seal", 52, 1000), ("seal", 52, 3000),
                                                 ("open", 52, 4000), ("seal", 1280, 5000)]]
        groups = {(row["operation"], row["bytes"]): row for row in crypto.aggregate(samples)}
        self.assertEqual(groups["seal", 52]["cpuNsPerOp"]["median"], 20)
        self.assertEqual(groups["seal", 1280]["cpuNsPerOp"]["median"], 50)
        self.assertEqual(groups["open", 52]["wallNsPerOp"]["median"], 80)

    def test_missing_or_invalid_cpu_is_not_zero(self):
        for value in [None, 0, -1, float("nan")]:
            with self.assertRaises(ValueError):
                crypto.aggregate([dict(operation="seal", bytes=52, iterations=100, cpuNs=value, wallNs=1)])


if __name__ == "__main__":
    unittest.main()
