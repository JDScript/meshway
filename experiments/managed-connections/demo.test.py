"""Offline recovery guards: real subprocess/network/Docker calls are mocked."""

import importlib.util
from contextlib import contextmanager
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import MagicMock, mock_open, patch

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


class ImageChecks(unittest.TestCase):
    def test_neighbors_do_not_require_packet_flags_or_capability_tools(self):
        with patch.object(demo, "checked", side_effect=["help", "--apply-connections --connection-profiles --connection-lease-seconds", "controller"]) as checked:
            self.assertEqual(demo.container_image_check({"shell": False}), {"managed": True, "ping_cli": False})
        self.assertEqual(checked.call_count, 3)

    def test_enable_requires_all_packet_cli_flags_before_any_state_or_controller_operation(self):
        managed = "--apply-connections --connection-profiles --connection-lease-seconds"
        flags = ["--apply-dataplane", "--tun-name", "--tun-mtu"]
        for missing in flags:
            with patch.object(demo, "checked", side_effect=["help", managed + " " + " ".join(flag for flag in flags if flag != missing)]) as checked:
                with self.assertRaisesRegex(RuntimeError, "IPv4 TUN"):
                    demo.container_image_check({"shell": False, "dataplane": True})
                self.assertEqual(checked.call_count, 2)
        with patch.object(demo, "checked", side_effect=["help", managed + " " + " ".join(flags), "setpriv", "ip", "controller"]) as checked:
            demo.container_image_check({"shell": False, "dataplane": True})
        self.assertEqual([call.args[0] for call in checked.call_args_list[2:4]], [["setpriv", "--version"], ["ip", "-Version"]])

    def test_running_exec_requires_original_uid_gid_zero_capabilities_and_readonly_rootfs(self):
        status = "CapEff:\t00000000\nCapPrm:\t00000000\nCapAmb:\t00000000\nNoNewPrivs:\t1\n"
        rootfs = "overlay / overlay ro,relatime 0 0\n"
        with patch.object(demo.os, "getuid", return_value=501), patch.object(demo.os, "geteuid", return_value=501), \
                patch.object(demo.os, "getgid", return_value=20), patch.object(demo.os, "getegid", return_value=20):
            with patch.object(demo.Path, "read_text", side_effect=[status, rootfs]):
                demo.running_exec_user(501, 20)
            for uid, gid in [(0, 20), (502, 20), (501, 21), (True, 20)]:
                with self.assertRaises(RuntimeError):
                    demo.running_exec_user(uid, gid)
            for changed in [status.replace("CapEff:\t00000000", "CapEff:\t00001000"),
                            status.replace("CapPrm:\t00000000", "CapPrm:\t00001000"),
                            status.replace("CapAmb:\t00000000", "CapAmb:\t00001000"), status.replace("Privs:\t1", "Privs:\t0")]:
                with patch.object(demo.Path, "read_text", side_effect=[changed, rootfs]):
                    with self.assertRaises(RuntimeError):
                        demo.running_exec_user(501, 20)
            with patch.object(demo.Path, "read_text", side_effect=[status, rootfs.replace("ro,", "rw,")]):
                with self.assertRaises(RuntimeError):
                    demo.running_exec_user(501, 20)

    def test_installed_old_agent_elf_architecture_is_read_without_execution_or_uname_inference(self):
        for machine, expected in [(183, "arm64"), (62, "amd64"), (3, None)]:
            header = bytearray(64); header[:6] = b"\x7fELF\x02\x01"; header[18:20] = machine.to_bytes(2, "little")
            with patch("builtins.open", mock_open(read_data=bytes(header))) as opened:
                if expected:
                    self.assertEqual(demo.running_agent_architecture(), expected)
                else:
                    with self.assertRaises(RuntimeError):
                        demo.running_agent_architecture()
                opened.assert_called_once_with("/usr/local/bin/meshway-agent", "rb")

    def test_stdin_loader_never_runs_demo_main_and_bounds_or_suppresses_invalid_private_input(self):
        loader = Path(__file__).with_name("running-check.py").read_text()
        code = 'if __name__ == "__main__":\n    raise RuntimeError("main must not run")\ndef container_running_check(data):\n    return {"node_id": data["node_id"]}\n'
        result = subprocess.run([sys.executable, "-c", loader], input=json.dumps({"code": code, "request": {"node_id": NODE_ID}}), text=True, capture_output=True, check=False)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout), {"node_id": NODE_ID})
        for envelope in [{"code": code, "request": {}, "unexpected": True}, {"code": "x" * (128 * 1024 + 1), "request": {}},
                         {"code": "raise RuntimeError('synthetic-private-value')", "request": {}}, {"code": "\x00", "request": {}}]:
            result = subprocess.run([sys.executable, "-c", loader], input=json.dumps(envelope), text=True, capture_output=True, check=False)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, "")
            self.assertNotIn("synthetic-private-value", result.stderr)


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

    def test_running_old_cli_uses_readonly_originals_and_disposable_private_tmp_copies(self):
        for failure in (None, "wrong-id", "wrong-key", "cli-error"):
            with self.subTest(failure=failure), self.fixture() as (source, scratch):
                original = self.fingerprint(source)
                actual_verify, actual_temporary, actual_open = demo.container_verify, demo.tempfile.TemporaryDirectory, demo.os.open
                source_reads = []

                def temporary(*args, **kwargs):
                    if kwargs.get("dir") == "/tmp":
                        kwargs["dir"] = scratch
                    return actual_temporary(*args, **kwargs)

                def verify(data, source_path, scratch_path):
                    self.assertEqual(source_path, "/state")
                    self.assertTrue(Path(scratch_path).name.startswith("meshway-old-verify-"))
                    self.assertEqual(Path(scratch_path).parent, scratch)
                    self.assertEqual(stat.S_IMODE(Path(scratch_path).stat().st_mode), 0o700)
                    return actual_verify(data, source_path=source, scratch_path=scratch_path)

                def source_open(path, flags, *args, **kwargs):
                    if path in demo.VERIFY_FILES and "dir_fd" in kwargs:
                        self.assertEqual(flags & os.O_ACCMODE, os.O_RDONLY)
                        self.assertTrue(flags & os.O_NOFOLLOW)
                        source_reads.append(path)
                    return actual_open(path, flags, *args, **kwargs)

                def cli(arguments):
                    state = Path(arguments[-1])
                    self.assertNotEqual(state.parent, source)
                    self.assertEqual(state.parent.parent.parent, scratch)
                    self.assertEqual(state.read_bytes(), (source / "agent.json").read_bytes())
                    with state.open("r+b") as copied:
                        copied.write(state.read_bytes())
                    if failure == "cli-error":
                        raise RuntimeError("synthetic CLI failure")
                    if arguments[1] == "status":
                        return json.dumps({"node_id": "3" * 64 if failure == "wrong-id" else NODE_ID, "name": "demo-sha", "controller": CONTROLLER})
                    self.assertEqual(arguments[1:3], ["connections", "status"])
                    return json.dumps({"public_key": "3" * 64 if failure == "wrong-key" else KEY})

                request = {"uid": os.getuid(), "gid": os.getgid(), "dataplane": True, "verify": self.data()}
                with patch.object(demo, "running_exec_user") as user, patch.object(demo, "running_agent_architecture", return_value="arm64"), \
                        patch.object(demo, "container_image_check", return_value={"managed": True, "ping_cli": True}) as image, \
                        patch.object(demo, "container_verify", side_effect=verify), patch.object(demo.tempfile, "TemporaryDirectory", side_effect=temporary), \
                        patch.object(demo.os, "open", side_effect=source_open), patch.object(demo, "checked", side_effect=cli):
                    if failure:
                        with self.assertRaises(RuntimeError):
                            demo.container_running_check(request)
                    else:
                        self.assertEqual(demo.container_running_check(request), {"managed": True, "ping_cli": True, "node_id": NODE_ID, "architecture": "arm64"})
                    user.assert_called_once_with(os.getuid(), os.getgid())
                    image.assert_called_once_with({"shell": False, "dataplane": True})
                self.assertTrue(source_reads)
                self.assertEqual(self.fingerprint(source), original)
                self.assertEqual(list(scratch.iterdir()), [])

    def test_running_helper_rejects_mutations_credentials_and_unbounded_public_request(self):
        request = {"uid": os.getuid(), "gid": os.getgid(), "dataplane": True, "verify": self.data()}
        for verify in [{**self.data(), "operation": "recover"}, {**self.data(), "operation": "publish"},
                       {**self.data(), "token": TOKEN}, {**self.data(), "node_credential": TOKEN},
                       {**self.data(), "expected_key": None}, {**self.data(), "expected_id": "wrong"},
                       {**self.data(), "controller": "http://user:private@host"}, {**self.data(), "controller": "x" * 2049}]:
            with patch.object(demo, "running_exec_user") as user, patch.object(demo, "container_image_check") as image:
                with self.assertRaises(RuntimeError):
                    demo.container_running_check({**request, "verify": verify})
                user.assert_not_called()
                image.assert_not_called()


if __name__ == "__main__":
    unittest.main()
