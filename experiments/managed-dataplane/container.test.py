#!/usr/bin/env python3
"""Host-only guard/proof regressions; no Docker, raw sockets or host network edits."""

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
APPROVED_TEMP = Path("/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode")
original_spec = importlib.util.spec_from_file_location


def local_spec(name, path):
    if path == "/opt/meshway-managed/container.py":
        path = HERE.parent / "managed-connections" / "container.py"
    return original_spec(name, path)


with patch("importlib.util.spec_from_file_location", side_effect=local_spec):
    spec = original_spec("fixture", HERE / "container.py")
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)


class GuardTests(unittest.TestCase):
    def lookup(self, result, routes=None):
        with patch.object(fixture, "run", return_value=result), patch.object(
                fixture.base, "checked", return_value=json.dumps(routes or [])):
            return fixture.route_observation("100.96.0.3")

    def test_underlay_success_cannot_masquerade_as_mesh_route(self):
        result = subprocess.CompletedProcess([], 0, '[{"dev":"eth0","prefsrc":"172.18.0.2"}]', "")
        self.assertEqual(self.lookup(result)["kind"], "underlay")

    def test_failed_lookup_requires_real_matching_drop_route(self):
        result = subprocess.CompletedProcess([], 2, "", "Network is unreachable")
        self.assertEqual(self.lookup(result)["kind"], "missing")
        self.assertEqual(self.lookup(result, [{"dst": "198.18.0.0/16", "type": "unreachable"}])["kind"], "missing")
        self.assertEqual(self.lookup(result, [{"dst": "default", "type": "unreachable"}])["kind"], "missing")
        self.assertEqual(self.lookup(result, [{"dst": "100.96.0.0/16", "dev": "eth0"}])["kind"], "missing")
        guarded = self.lookup(result, [{"dst": "100.96.0.0/16", "type": "unreachable", "metric": 42760}])
        self.assertEqual(guarded["kind"], "guard")
        self.assertEqual(guarded["guard"], {"prefix": "100.96.0.0/16", "type": "unreachable"})

    def test_live_tun_is_distinct_from_dead_guard(self):
        result = subprocess.CompletedProcess([], 0, '[{"dev":"meshway0","prefsrc":"100.96.0.1"}]', "")
        self.assertEqual(self.lookup(result)["kind"], "tun")

    def test_probe_addresses_cannot_use_underlay_loopback_or_lab_range(self):
        for address in ("127.0.0.1", "172.18.0.3", "198.18.0.3", "::1", "example.org"):
            with self.assertRaises(ValueError):
                fixture.mesh_ip(address)
        self.assertEqual(fixture.mesh_ip("100.96.0.3"), "100.96.0.3")

    def test_latest_withdrawal_supersedes_old_ready_and_does_not_export_logs(self):
        event = {"event": "managed_dataplane", "node_id": "a" * 64, "tun_name": "meshway0",
                 "revision": 17, "digest": "d" * 64,
                 "ipv4": "100.96.0.1", "mtu": 1000, "ipv6": False, "tun_ready": True,
                 "allowed_paths": 2, "route_sink_retained": True, "reason": "generation_active",
                 "private_unknown_field": "never-export-this"}
        with tempfile.TemporaryDirectory(dir=APPROVED_TEMP if APPROVED_TEMP.is_dir() else None) as directory, patch.object(fixture, "ROOT", Path(directory)):
            path = Path(directory) / "agent.log"
            path.write_text('unstructured secret error\n' + json.dumps(event) + '\n')
            public, count = fixture.dataplane_proof()
            self.assertTrue(public["tun_ready"])
            self.assertEqual(public["revision"], 17)
            self.assertEqual(public["digest"], "d" * 64)
            self.assertEqual(count, 1)
            self.assertNotIn("never-export-this", json.dumps(public))
            with path.open("a") as output:
                output.write(json.dumps({**event, "tun_ready": False, "allowed_paths": 0, "reason": "lease_expired"}) + '\n')
            public, count = fixture.dataplane_proof()
            self.assertFalse(public["tun_ready"])
            self.assertEqual(public["reason"], "lease_expired")
            self.assertEqual(count, 1)

    def test_missing_or_invalid_latest_binding_never_reuses_a_historical_proof(self):
        event = {"event": "managed_dataplane", "node_id": "a" * 64, "revision": 17, "digest": "d" * 64,
                 "tun_name": "meshway0", "ipv4": "100.96.0.1", "mtu": 1000, "ipv6": False,
                 "tun_ready": True, "allowed_paths": 2, "route_sink_retained": True, "reason": "generation_active"}
        invalid = [{key: value for key, value in event.items() if key != missing} for missing in ("revision", "digest")]
        invalid.extend({**event, "revision": value} for value in (None, True, 0, -1, 1.5, "17", 0x100000000))
        invalid.extend({**event, "digest": value} for value in (None, 17, "", "d" * 63, "D" * 64, "x" * 64, "private-text"))
        with tempfile.TemporaryDirectory(dir=APPROVED_TEMP if APPROVED_TEMP.is_dir() else None) as directory, patch.object(fixture, "ROOT", Path(directory)):
            path = Path(directory) / "agent.log"
            for candidate in invalid:
                with self.subTest(revision=candidate.get("revision"), digest=candidate.get("digest")):
                    path.write_text(json.dumps(event) + '\n' + json.dumps(candidate) + '\n')
                    public, count = fixture.dataplane_proof()
                    self.assertIsNone(public)
                    self.assertEqual(count, 1)
            path.write_text(json.dumps(invalid[0]) + '\n')
            self.assertEqual(fixture.dataplane_proof(), (None, 0))
            with path.open("a") as output:
                output.write(json.dumps(event) + '\n')
            public, count = fixture.dataplane_proof()
            self.assertEqual(public["revision"], 17)
            self.assertEqual(public["digest"], "d" * 64)
            self.assertEqual(count, 1)


if __name__ == "__main__":
    unittest.main()
