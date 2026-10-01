"""Offline recovery guards: real subprocess/network/Docker calls are mocked."""

import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import MagicMock, patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("meshway_demo", Path(__file__).with_name("demo.py"))
assert spec and spec.loader
demo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(demo)

NODE_ID = "1" * 64
KEY = "2" * 64
TOKEN = "a" * 64
CONTROLLER = "http://host.docker.internal:8420"


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.state = MagicMock()
        self.state.exists.return_value = True
        self.state.is_symlink.return_value = False
        self.path_patch = patch.object(demo, "Path", return_value=self.state)
        self.path_patch.start()
        self.uid_patch = patch.object(demo.os, "getuid", return_value=501)
        self.uid_patch.start()
        self.mask_patch = patch.object(demo.os, "umask")
        self.mask_patch.start()
        self.addCleanup(self.path_patch.stop)
        self.addCleanup(self.uid_patch.stop)
        self.addCleanup(self.mask_patch.stop)

    def data(self, **overrides):
        return {"operation": "recover", "name": "demo-sha", "controller": CONTROLLER,
                "token": TOKEN, "expected_id": None, **overrides}

    def status(self, node_id):
        return json.dumps({"name": "demo-sha", "controller": CONTROLLER, "node_id": node_id})

    def test_lost_enrollment_response_recovers_original_credential_by_heartbeat(self):
        with patch.object(demo, "checked", side_effect=[self.status(None), "{}", self.status(NODE_ID)]) as checked:
            self.assertEqual(demo.container_enroll(self.data()), {"node_id": NODE_ID})
        self.assertEqual([call.args[0][1] for call in checked.call_args_list], ["status", "heartbeat", "status"])

    def test_pending_uncommitted_enrollment_reuses_original_grant_and_binding(self):
        with patch.object(demo, "checked", side_effect=[self.status(None), RuntimeError("offline"), "{}", self.status(NODE_ID)]) as checked:
            self.assertEqual(demo.container_enroll(self.data()), {"node_id": NODE_ID})
        enrollment = checked.call_args_list[2].args[0]
        self.assertEqual(enrollment[1], "enroll")
        self.assertEqual(enrollment[enrollment.index("--token") + 1], TOKEN)
        self.assertEqual(enrollment[enrollment.index("--controller") + 1], CONTROLLER)
        self.assertEqual(enrollment[enrollment.index("--name") + 1], "demo-sha")

    def test_enrolled_unknown_or_revoked_identity_never_falls_back_to_enrollment(self):
        with patch.object(demo, "checked", side_effect=[self.status(NODE_ID), RuntimeError("denied")]) as checked:
            with self.assertRaises(RuntimeError):
                demo.container_enroll(self.data(expected_id=NODE_ID))
        self.assertEqual([call.args[0][1] for call in checked.call_args_list], ["status", "heartbeat"])

    def test_pending_without_original_grant_does_not_invent_replacement(self):
        with patch.object(demo, "checked", side_effect=[self.status(None), RuntimeError("denied")]) as checked:
            with self.assertRaises(RuntimeError):
                demo.container_enroll(self.data(token=None))
        self.assertEqual(checked.call_count, 2)

    def test_different_identity_or_controller_is_rejected_before_network_use(self):
        for status in [self.status("3" * 64), json.dumps({"name": "demo-sha", "controller": "http://different:8420", "node_id": NODE_ID})]:
            with patch.object(demo, "checked", return_value=status) as checked:
                with self.assertRaises(RuntimeError):
                    demo.container_enroll(self.data(expected_id=NODE_ID))
            self.assertEqual(checked.call_count, 1)

    def test_missing_or_changed_bound_noise_key_is_not_regenerated(self):
        for key in [None, "4" * 64]:
            with patch.object(demo, "checked", side_effect=[self.status(NODE_ID), json.dumps({"public_key": key})]) as checked:
                with self.assertRaises(RuntimeError):
                    demo.container_enroll(self.data(operation="publish", expected_id=NODE_ID, expected_key=KEY, token=None))
            self.assertEqual([call.args[0][1:3] for call in checked.call_args_list], [["status", "--state"], ["connections", "status"]])

    def test_publish_reuses_valid_bound_key(self):
        with patch.object(demo, "checked", side_effect=[self.status(NODE_ID), json.dumps({"public_key": KEY}), "{}"]) as checked:
            self.assertEqual(demo.container_enroll(self.data(operation="publish", expected_id=NODE_ID, expected_key=KEY, token=None)), {"node_id": NODE_ID})
        self.assertEqual(checked.call_args_list[-1].args[0][1:3], ["connections", "publish"])


if __name__ == "__main__":
    unittest.main()
