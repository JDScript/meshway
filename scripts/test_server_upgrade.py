"""The upgrade test's reading of a Node status and of a database header, and
its failure path: every process stopped, evidence kept, never `passed`."""
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "ci" / "server-upgrade.py"
spec = importlib.util.spec_from_file_location("server_upgrade", SCRIPT)
upgrade = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upgrade)


class ServerUpgradeTests(unittest.TestCase):
    def test_the_held_lease_is_read_in_unix_milliseconds(self):
        status = {"netmap": {"issuedAt": "2026-10-22T09:26:48.725Z"}}
        self.assertEqual(upgrade.issued_ms(status), 1792661208725)
        self.assertEqual(upgrade.issued_ms({"netmap": {"issuedAt": "2026-10-22T09:26:48Z"}}), 1792661208000)
        self.assertEqual(upgrade.issued_ms({"netmap": None}), -1)
        self.assertEqual(upgrade.issued_ms(None), -1)

    def test_the_schema_is_the_user_version_of_the_header(self):
        with tempfile.TemporaryDirectory() as directory:
            header = bytearray(100)
            header[60:64] = (9).to_bytes(4, "big")
            (Path(directory) / "controller.sqlite").write_bytes(bytes(header))
            self.assertEqual(upgrade.database_schema(Path(directory)), 9)

    def test_a_run_that_stops_early_stops_every_process_and_does_not_pass(self):
        class Started:
            name = "server-started-before-the-failure"

            def __init__(self):
                self.process = subprocess.Popen(["sleep", "60"])

            def stop(self):
                self.process.terminate()
                return self.process.wait(timeout=10)

        started = Started()
        upgrade.STARTED.append(started)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / "report.json"
            argv = sys.argv
            sys.argv = ["server-upgrade.py", "--previous-tag", "v0.0.0", "--previous-server", str(root / "absent"),
                        "--previous-node-bin", str(root), "--work", str(root / "work"), "--report", str(report)]
            try:
                self.assertEqual(upgrade.main(), 1)
            finally:
                sys.argv = argv
                upgrade.STARTED.clear()
            self.assertIsNotNone(started.process.poll(), "a started process was left running")
            written = json.loads(report.read_text())
            self.assertFalse(written["passed"])
            self.assertFalse(written["completed"])
            self.assertIn("error", written)
            self.assertEqual(written["missing"], upgrade.REQUIRED)
            self.assertIn("server-started-before-the-failure", written["stopped"])
            # The work directory is kept as evidence.
            self.assertTrue((root / "work" / "logs").is_dir())


if __name__ == "__main__":
    unittest.main()
