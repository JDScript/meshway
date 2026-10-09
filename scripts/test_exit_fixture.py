import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("exit_evidence", Path(__file__).resolve().parents[1] / "dev/exit-fixture/evidence.py")
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)


class ExitEvidenceTests(unittest.TestCase):
    def test_tuple_requires_original_and_reverse_nat_and_owned_mark(self):
        line = "udp 17 29 src=100.64.0.1 dst=11.77.2.3 sport=40000 dport=8080 src=11.77.2.3 dst=11.77.2.2 sport=8080 dport=41000 mark=8 use=1"
        self.assertEqual(evidence.nat_tuple(line, "udp", "100.64.0.1:40000", "11.77.2.3:8080", "11.77.2.2")["reply"]["dport"], "41000")
        self.assertEqual(evidence.nat_tuple("ipv4     2 " + line, "udp", "100.64.0.1:40000", "11.77.2.3:8080", "11.77.2.2")["mark"], 8)
        for invalid in (line.replace("mark=8", "mark=0"), line.replace("dst=11.77.2.2", "dst=100.64.0.1"),
                        line.replace("sport=40000", "sport=40001"), line + "\n" + line):
            with self.assertRaises(RuntimeError):
                evidence.nat_tuple(invalid, "udp", "100.64.0.1:40000", "11.77.2.3:8080", "11.77.2.2")

    def test_component_or_partial_pass_cannot_promote_row(self):
        report = {"stages": {str(i): {"status": "passed"} for i in range(1, 9)},
                  "processes": {role: {"ExitCode": 0, "Running": False, "OOMKilled": False}
                                for role in ("controller", "a", "r", "e", "external", "lan")}}
        self.assertTrue(evidence.all_stages_passed(report))
        report["stages"]["7"]["status"] = "observed"
        self.assertFalse(evidence.all_stages_passed(report))
        report["stages"]["7"]["status"] = "passed"
        report["cleanupErrors"] = ["namespace still running"]
        self.assertFalse(evidence.all_stages_passed(report))


if __name__ == "__main__":
    unittest.main()
