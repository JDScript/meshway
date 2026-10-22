"""The upgrade test's reading of a Node status and of a database header."""
import importlib.util
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


if __name__ == "__main__":
    unittest.main()
