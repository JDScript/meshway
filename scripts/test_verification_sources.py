"""Source provenance regressions on temporary trees; no Cargo or Docker."""
import json
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch

import verification_sources as sources
import verify_packet
import verify_runtime

REPOSITORY = Path(__file__).resolve().parent.parent


def local_dependencies(manifest, dev):
    """Path dependencies of one package; dev-dependencies only for --all-targets roots."""
    tables = ["dependencies", "build-dependencies"] + (["dev-dependencies"] if dev else [])
    sections = [manifest.get(name, {}) for name in tables]
    sections += [target.get(name, {}) for target in manifest.get("target", {}).values() for name in tables]
    return {spec["path"] for section in sections for spec in section.values()
            if isinstance(spec, dict) and "path" in spec}


def build_closure(packages, dev_roots):
    members = tomllib.loads((REPOSITORY / "Cargo.toml").read_text())["workspace"]["members"]
    by_name = {}
    for member in members:
        manifest = tomllib.loads((REPOSITORY / member / "Cargo.toml").read_text())
        by_name[manifest["package"]["name"]] = (REPOSITORY / member).resolve()
    pending = [(by_name[name], dev_roots) for name in packages]
    seen = set()
    while pending:
        directory, dev = pending.pop()
        if directory in seen and not dev:
            continue
        seen.add(directory)
        manifest = tomllib.loads((directory / "Cargo.toml").read_text())
        pending.extend(((directory / path).resolve(), False) for path in local_dependencies(manifest, dev))
    return {directory.relative_to(REPOSITORY).as_posix() for directory in seen}


class BuildClosure(unittest.TestCase):
    def assert_covered(self, packages, dev_roots):
        for directory in sorted(build_closure(packages, dev_roots)):
            with self.subTest(directory=directory):
                self.assertTrue(any(directory == tree or directory.startswith(tree + "/") for tree in sources.TREES),
                                directory + " is built but not part of the source fingerprint")

    def test_runtime_all_targets_closure_is_fingerprinted(self):
        self.assert_covered(verify_runtime.PACKAGES, dev_roots=True)

    def test_packet_fixture_closure_is_fingerprinted(self):
        self.assert_covered(("meshway-packet-fixture",), dev_roots=False)


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

    def test_changed_test_server_rejects_previous_runtime_manifest(self):
        fixture = self.write("tools/meshway-test-server/src/lib.rs", b"pub fn first() {}\n")
        with patch.object(verify_runtime, "ROOT", self.root):
            previous = self.root / "previous"
            previous.mkdir()
            (previous / "source-manifest.json").write_text(json.dumps({
                "sourceDigest": verify_runtime.fingerprint(), "target": "x86_64-unknown-linux-gnu",
                "platform": "linux/amd64", "sha256": {name: "" for name in verify_runtime.ARTIFACTS},
                "exitFixtureSources": {},
            }))
            fixture.write_bytes(b"pub fn second() {}\n")
            with patch.object(verify_runtime, "check_elf", side_effect=AssertionError("digest must be checked first")):
                with self.assertRaisesRegex(RuntimeError, "Prebuilt manifest does not match"):
                    verify_runtime.import_artifacts(previous, self.root / "bin", "x86_64-unknown-linux-gnu",
                                                    "linux/amd64", "x86_64", verify_runtime.fingerprint())


if __name__ == "__main__":
    unittest.main()
