"""Source provenance regressions on temporary trees; no Cargo or Docker."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import verification_sources as sources
import verify_packet
import verify_runtime


class Fingerprint(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="meshway-source-inputs-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        for name in sources.TREES:
            (self.root / name).mkdir(parents=True)
        for name in sources.FILES:
            self.write(name, b"original\n")

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def test_sql_binary_assets_build_scripts_and_tool_locks_change_identity(self):
        for name in (
            "apps/meshway-server/src/store/schema/current.sql",
            "crates/meshway-example/data/include.bin",
            "crates/meshway-example/build.rs",
            "dev/dns-fixture/assets.lock.json",
            "dev/traversal-fixture/evidence.py",
            *sources.FILES,
        ):
            with self.subTest(name=name):
                path = self.write(name, b"first\n")
                before = sources.fingerprint(self.root)
                path.write_bytes(b"second\n")
                self.assertNotEqual(sources.fingerprint(self.root), before)

    def test_add_remove_and_rename_inputs_change_identity(self):
        before = sources.fingerprint(self.root)
        path = self.write("crates/meshway-example/input.sql", b"schema\n")
        added = sources.fingerprint(self.root)
        self.assertNotEqual(added, before)
        renamed = path.with_name("renamed.sql")
        path.rename(renamed)
        self.assertNotEqual(sources.fingerprint(self.root), added)
        renamed.unlink()
        self.assertEqual(sources.fingerprint(self.root), before)

    def test_cache_and_private_runtime_evidence_do_not_change_identity(self):
        before = sources.fingerprint(self.root)
        for name in (
            "crates/example/target/debug/binary", "crates/example/__pycache__/cache.pyc",
            "apps/meshway-daemon/node_modules/cache", "crates/example/.git/index",
            "apps/meshway-server/.DS_Store", ".meshway/validation/run/report.json",
        ):
            self.write(name, b"runtime output")
        self.assertEqual(sources.fingerprint(self.root), before)

    def test_required_inputs_cannot_silently_disappear(self):
        (self.root / "mise.lock").unlink()
        with self.assertRaises(FileNotFoundError):
            sources.fingerprint(self.root)

    def test_both_verification_entrypoints_use_the_same_rules(self):
        with patch.object(verify_packet, "ROOT", self.root), patch.object(verify_runtime, "ROOT", self.root):
            before = verify_packet.fingerprint()
            self.assertEqual(before, verify_runtime.fingerprint())
            self.write("apps/meshway-server/src/store/schema/state.sql", b"changed schema")
            self.assertNotEqual(before, verify_packet.fingerprint())
            self.assertEqual(verify_packet.fingerprint(), verify_runtime.fingerprint())


if __name__ == "__main__":
    unittest.main()
