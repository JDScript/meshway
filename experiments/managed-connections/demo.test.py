"""Offline recovery guards: real subprocess/network/Docker calls are mocked."""

import importlib.util
from contextlib import contextmanager
import json
import os
from pathlib import Path
import socket
import stat
import sys
import tempfile
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

    def test_upgrade_verify_is_read_only_with_explicit_state_and_no_enrollment_or_publication(self):
        with patch.object(demo.os, "environ", {"MESHWAY_STATE": "/different/fixture.json"}):
            with patch.object(demo, "checked", side_effect=[self.status(NODE_ID), json.dumps({"public_key": KEY})]) as checked:
                result = demo.container_enroll(self.data(operation="verify", expected_id=NODE_ID, expected_key=KEY, token=None))
        self.assertEqual(result, {"node_id": NODE_ID})
        self.assertEqual([call.args[0][1:3] for call in checked.call_args_list], [["status", "--state"], ["connections", "status"]])
        for call in checked.call_args_list:
            self.assertEqual(call.args[0][-2], "--state")

    def test_upgrade_verify_rejects_changed_identity_or_key_without_mutation(self):
        for responses in [[self.status("3" * 64)], [self.status(NODE_ID), json.dumps({"public_key": "4" * 64})]]:
            with patch.object(demo, "checked", side_effect=responses) as checked:
                with self.assertRaises(RuntimeError):
                    demo.container_enroll(self.data(operation="verify", expected_id=NODE_ID, expected_key=KEY, token=None))
            self.assertTrue(all(call.args[0][1] in ("status", "connections") for call in checked.call_args_list))


class PrivateCopyTests(unittest.TestCase):
    @contextmanager
    def fixture(self, caches=True):
        parent = Path(tempfile.gettempdir()) / "opencode"
        parent.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="mvc-", dir=parent) as root:
            source, scratch = Path(root) / "s", Path(root) / "w"
            source.mkdir(mode=0o700)
            scratch.mkdir(mode=0o700)
            for name, (required, _) in demo.VERIFY_FILES.items():
                if required or caches:
                    (source / name).write_bytes((name + " synthetic private bytes\n").encode())
                    (source / name).chmod(0o600)
            yield source, scratch

    def data(self):
        return {"operation": "verify", "name": "demo-sha", "controller": CONTROLLER,
                "token": None, "expected_id": NODE_ID, "expected_key": KEY}

    def fingerprint(self, source):
        return {path.name: (path.read_bytes(), path.stat().st_ino, path.stat().st_mode, path.stat().st_mtime_ns)
                for path in source.iterdir() if path.is_file()}

    def test_actual_private_copies_are_writable_but_sources_and_socket_are_untouched(self):
        with self.fixture() as (source, scratch), socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as control:
            # Relative binding avoids the macOS Unix-domain path length limit.
            cwd = os.getcwd()
            try:
                os.chdir(source)
                control.bind("agent.json.control.sock")
            finally:
                os.chdir(cwd)
            (source / "profiles.json").write_text("unrelated material")
            (source / "profiles.json").chmod(0o600)
            original = self.fingerprint(source)
            actual_open = demo.os.open
            source_reads = []

            def checked_open(path, flags, *args, **kwargs):
                if path in demo.VERIFY_FILES and "dir_fd" in kwargs:
                    self.assertEqual(flags & os.O_ACCMODE, os.O_RDONLY)
                    self.assertTrue(flags & os.O_NOFOLLOW)
                    source_reads.append(path)
                return actual_open(path, flags, *args, **kwargs)

            def cli(arguments):
                state = Path(arguments[-1])
                self.assertEqual(arguments[-2], "--state")
                self.assertEqual(state.parent.parent, scratch)
                self.assertEqual(stat.S_IMODE(state.parent.stat().st_mode), 0o700)
                self.assertEqual(set(path.name for path in state.parent.iterdir()), set(demo.VERIFY_FILES))
                for name in demo.VERIFY_FILES:
                    copy = state.parent / name
                    self.assertEqual(stat.S_IMODE(copy.stat().st_mode), 0o600)
                    self.assertNotEqual(copy.stat().st_ino, (source / name).stat().st_ino)
                    self.assertEqual(copy.read_bytes(), (source / name).read_bytes())
                    # Match the actual protocol primitive's read+write open.
                    with copy.open("r+b") as writable:
                        writable.write(copy.read_bytes())
                if arguments[1] == "status":
                    return json.dumps({"node_id": NODE_ID, "name": "demo-sha", "controller": CONTROLLER})
                self.assertEqual(arguments[1:3], ["connections", "status"])
                return json.dumps({"public_key": KEY})

            with patch.object(demo.os, "open", side_effect=checked_open), patch.object(demo, "checked", side_effect=cli) as checked:
                self.assertEqual(demo.container_verify(self.data(), source, scratch), {"node_id": NODE_ID})
            self.assertEqual(checked.call_count, 2)
            self.assertTrue(source_reads)
            self.assertEqual(self.fingerprint(source), original)
            self.assertTrue(stat.S_ISSOCK((source / "agent.json.control.sock").lstat().st_mode))
            self.assertEqual(list(scratch.iterdir()), [], "copies must be discarded")

    def test_failed_real_cli_discards_copies_without_writing_originals(self):
        with self.fixture() as (source, scratch):
            original = self.fingerprint(source)
            with patch.object(demo, "checked", side_effect=RuntimeError("invalid state")):
                with self.assertRaises(RuntimeError):
                    demo.container_verify(self.data(), source, scratch)
            self.assertEqual(self.fingerprint(source), original)
            self.assertEqual(list(scratch.iterdir()), [])

    def test_symlinks_hardlinks_special_files_public_modes_and_foreign_owners_reject_before_cli(self):
        for kind in ("symlink", "hardlink", "fifo", "mode", "owner", "cache-symlink"):
            with self.subTest(kind=kind), self.fixture() as (source, scratch):
                name = "agent.json.config.json" if kind == "cache-symlink" else "agent.json"
                path = source / name
                if kind in ("symlink", "cache-symlink"):
                    path.unlink()
                    path.symlink_to(source / "agent.json.connection-key.json")
                elif kind == "hardlink":
                    os.link(path, source / "unrelated-hardlink")
                elif kind == "fifo":
                    path.unlink()
                    os.mkfifo(path, 0o600)
                elif kind == "mode":
                    path.chmod(0o644)
                actual_stat = demo.os.fstat

                def inspected(fd):
                    value = actual_stat(fd)
                    if kind == "owner" and stat.S_ISREG(value.st_mode):
                        values = list(value)
                        values[4] = os.getuid() + 1
                        return os.stat_result(values)
                    return value

                with patch.object(demo.os, "fstat", side_effect=inspected), patch.object(demo, "checked") as checked:
                    with self.assertRaises((RuntimeError, OSError)):
                        demo.container_verify(self.data(), source, scratch)
                    checked.assert_not_called()
                self.assertEqual(list(scratch.iterdir()), [])

    def test_optional_absent_caches_are_not_invented(self):
        with self.fixture(caches=False) as (source, scratch):
            def cli(arguments):
                copied = Path(arguments[-1]).parent
                self.assertEqual(set(path.name for path in copied.iterdir()), {"agent.json", "agent.json.connection-key.json"})
                if arguments[1] == "status":
                    return json.dumps({"node_id": NODE_ID, "name": "demo-sha", "controller": CONTROLLER})
                return json.dumps({"public_key": KEY})
            with patch.object(demo, "checked", side_effect=cli):
                self.assertEqual(demo.container_verify(self.data(), source, scratch), {"node_id": NODE_ID})


if __name__ == "__main__":
    unittest.main()
