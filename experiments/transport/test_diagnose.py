import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, MagicMock

import diagnose


class DiagnosticFailureTests(unittest.TestCase):
    def test_iperf_failure_without_rate_is_saved_as_failed_and_cleans_up(self):
        fixture = MagicMock()
        fixture.setup_mesh.return_value = {}
        fixture.snapshot.return_value = {}
        fixture.kernel.return_value = {}
        fixture.statuses.return_value = {}
        fixture.cgroups.return_value = {}
        fixture.cleanup.return_value = []
        case = {
            "id": "tcp-p1-forward-1", "status": "failed",
            "summary": {"receiver_bits_per_second": None, "retransmits": None},
            "raw_client": {"error": "connection reset by peer"},
        }
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "failed.json"
            with (patch.object(diagnose, "DiagnosticFixture", return_value=fixture),
                  patch.object(diagnose, "docker_json", return_value={"OSType": "linux", "Architecture": "aarch64"}),
                  patch.object(diagnose.platform, "machine", return_value="aarch64"),
                  patch.object(diagnose, "benchmark_case", return_value=case),
                  patch.object(diagnose.signal, "signal"),
                  contextlib.redirect_stdout(io.StringIO()),
                  contextlib.redirect_stderr(io.StringIO())):
                self.assertEqual(diagnose.main(["--output", str(output)]), 1)
            result = json.loads(output.read_text())
            self.assertEqual(result["status"], "failed")
            self.assertEqual(result["cases"][0]["raw_client"]["error"], "connection reset by peer")
            self.assertEqual(result["error"], "iperf3 diagnostic failed")
            fixture.cleanup.assert_called_once()


if __name__ == "__main__":
    unittest.main()
