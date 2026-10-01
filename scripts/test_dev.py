"""Host-only behavioral tests. No live tmux, Docker, controller or network changes."""
from contextlib import contextmanager, redirect_stdout
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shlex
import signal
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import dev
import dev_demo_update as deployment


def result(stdout="", returncode=0, stderr=""):
    return subprocess.CompletedProcess([], returncode, stdout, stderr)


def executable(machine=183, content=b"new"):
    header = bytearray(64)
    header[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<H", header, 16, 3)
    struct.pack_into("<H", header, 18, machine)
    return bytes(header) + content


class Workspace(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="meshway dev 'quoted' ")
        self.root = Path(self.temp.name).resolve()
        (self.root / ".meshway").mkdir(mode=0o700)
        (self.root / ".meshway/services").mkdir(mode=0o700)
        self.local = dev.LocalServices(self.root, wait=0)
        self.artifacts = patch.object(deployment, "ARTIFACT_PARENT", self.root)
        self.artifacts.start()

    def tearDown(self):
        self.artifacts.stop()
        self.temp.cleanup()

    def pane_output(self, command=None, cwd=None):
        shell = command or "exec " + shlex.join(dev.command("controller", self.root)) + " >>" + shlex.quote(str(self.local.directory / "meshway-controller.log")) + " 2>&1"
        return result("meshway-controller\t%8\t101\t" + str(cwd or self.root) + "\t" + shell + "\n")

    def test_adoption_checks_cwd_start_and_actual_process(self):
        actual = " ".join(dev.command("controller", self.root))
        with patch.object(dev, "run", return_value=self.pane_output()), patch.object(dev, "process_table", return_value={101: (1, "S", actual), 102: (101, "S", "worker")}):
            self.assertEqual(self.local.pane("controller")["processes"], {101, 102})
        for output, raw in ((self.pane_output(cwd=self.root.parent), actual), (self.pane_output(command="exec sleep 1000"), actual), (self.pane_output(), "foreign bearer=SECRET")):
            with patch.object(dev, "run", return_value=output), patch.object(dev, "process_table", return_value={101: (1, "S", raw)}):
                with self.assertRaises(dev.DevError):
                    self.local.pane("controller")

    def test_shell_suffix_and_foreign_log_are_rejected(self):
        log = self.local.directory / "meshway-web.log"
        base = "exec env " + " ".join(key + "=" + value for key, value in dev.WEB_ENV.items()) + " pnpm dev:web >>" + shlex.quote(str(log)) + " 2>&1"
        self.assertTrue(dev.known_start(base, "web", self.root, log))
        self.assertFalse(dev.known_start(base + " ; sleep 1", "web", self.root, log))
        self.assertFalse(dev.known_start(base.replace("NEXT_TELEMETRY_DISABLED=1", "PRIVATE_TOKEN=SECRET"), "web", self.root, log))
        self.assertFalse(dev.known_start(base.replace(shlex.quote(str(log)), shlex.quote(str(self.root / "foreign.log"))), "web", self.root, log))

    def test_foreign_port_blocks_start_without_spawn(self):
        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "listeners", return_value={777}), patch.object(dev, "run") as child:
            with self.assertRaisesRegex(dev.DevError, "foreign listener"):
                self.local.start("controller")
            child.assert_not_called()

    def test_idempotent_start_records_ownership_without_new_session(self):
        pane = {"pane": "%8", "pid": 101, "processes": {101}}
        with patch.object(self.local, "pane", return_value=pane), patch.object(self.local, "port", return_value=True), patch.object(self.local, "health", return_value=True), patch.object(dev, "run") as child:
            self.local.start("controller")
            self.local.start("controller")
            child.assert_not_called()
        saved = json.loads((self.local.directory / "controller.json").read_text())
        self.assertEqual(saved["root"], str(self.root))
        dev.private(self.local.directory / "controller.json")

    def test_stop_signals_owned_tree_only_and_times_out_without_force(self):
        pane = {"pane": "%8", "pid": 101, "processes": {101, 102}}
        with patch.object(self.local, "pane", return_value=pane), patch.object(self.local, "port", return_value=True), patch.object(dev, "process_table", return_value={101: (1, "S", "controller"), 102: (101, "S", "worker"), 777: (1, "S", "foreign")}), patch.object(dev.os, "kill") as kill, patch.object(dev, "run") as child:
            with self.assertRaisesRegex(dev.DevError, "SIGINT timeout"):
                self.local.stop("controller")
            self.assertEqual({call.args for call in kill.call_args_list}, {(101, signal.SIGINT), (102, signal.SIGINT)})
            child.assert_not_called()

    def test_graceful_stop_removes_only_own_state(self):
        pane = {"pane": "%8", "pid": 101, "processes": {101}}
        path = self.local.directory / "controller.json"
        dev.save(path, {"service": "controller", "root": str(self.root)})
        with patch.object(self.local, "pane", side_effect=[pane, pane, None]), patch.object(self.local, "port", return_value=True), patch.object(dev, "process_table", return_value={777: (1, "S", "foreign")}), patch.object(dev.os, "kill"):
            self.local.stop("controller")
        self.assertFalse(path.exists())

    def test_startup_timeout_and_status_keep_running_distinct_from_ready(self):
        pane = {"pane": "%8", "pid": 101, "processes": {101}}
        with patch.object(self.local, "pane", return_value=pane), patch.object(self.local, "port", return_value=True), patch.object(self.local, "health", return_value=False):
            with self.assertRaisesRegex(dev.DevError, "readiness deadline"):
                self.local.start("controller")
            row = self.local.status("controller")
            self.assertEqual(row["session"], "running")
            self.assertFalse(row["ready"])
            self.assertTrue(row["listening"])

    def test_status_suppresses_untrusted_exception_and_secret(self):
        with patch.object(self.local, "pane", side_effect=dev.DevError("bearer SECRET")):
            row = self.local.status("controller")
        self.assertNotIn("SECRET", json.dumps(row))
        self.assertFalse(row["ready"])

    def test_new_session_uses_argument_safe_paths_and_private_logs(self):
        (self.root / "target/debug").mkdir(parents=True)
        (self.root / "target/debug/meshway-controller").write_bytes(b"binary")
        pane = {"pane": "%8", "pid": 101, "processes": {101}}
        with patch.object(self.local, "pane", side_effect=[None, pane]), patch.object(self.local, "port", side_effect=[False, True]), patch.object(self.local, "health", return_value=True), patch.object(dev, "run", return_value=result()) as child:
            self.local.start("controller")
        args = child.call_args.args[0]
        self.assertEqual(args[args.index("-c") + 1], str(self.root))
        self.assertTrue(dev.known_start(args[-1], "controller", self.root, self.local.directory / "meshway-controller.log"))
        dev.private(self.local.directory / "meshway-controller.log")

    def make_demo(self):
        demo = dev.Demo(self.root)
        demo.data.mkdir(mode=0o700)
        meta = {"version": 1, "run_id": "a" * 32, "fingerprint": hashlib.sha256(str(demo.data).encode()).hexdigest(), "uid": os.getuid(), "gid": os.getgid(), "phase": "ready", "shell": "development", "mode": "linux-ipv4-tun", "image_id": "sha256:" + "b" * 64,
                "controller": "http://127.0.0.1:8420", "agent_controller": "http://host.docker.internal:8420", "host_gateway": False}
        dev.save(demo.data / "demo.json", meta)
        for role in deployment.ROLES:
            (demo.data / role).mkdir(mode=0o700)
            for name in ("agent.json", "agent.json.connection-key.json", "profiles.json", "private-key.pem"):
                path = demo.data / role / name
                path.write_text("secret-material " + role + " " + name)
                path.chmod(0o600)
        return demo, meta

    def test_demo_status_failure_cannot_be_reported_ready_and_filters_secrets(self):
        demo, _ = self.make_demo()
        value = {"nodes": [{"name": "demo-" + role, "container": "running", "dataplane": "ready", "registration": "active", "runtime": {"current": True, "stale": False, "bearer": "SECRET"}, "token": "SECRET"} for role in deployment.ROLES], "admin": "SECRET"}
        with patch.object(demo, "invoke", return_value=result(json.dumps(value), returncode=1)):
            row = demo.status()
        self.assertEqual(row["session"], "running")
        self.assertFalse(row["ready"])
        self.assertNotIn("SECRET", json.dumps(row))

    def test_demo_status_accepts_lock_wait_notice_and_rejects_malformed_report(self):
        demo, _ = self.make_demo()
        value = {"nodes": [{"name": "demo-" + role, "container": "running", "dataplane": "ready", "registration": "active", "runtime": {"current": True, "stale": False}} for role in deployment.ROLES]}
        with patch.object(demo, "invoke", return_value=result("Waiting for the shared Meshway performance runner…\n" + json.dumps(value))):
            self.assertTrue(demo.status()["ready"])
        with patch.object(demo, "invoke", return_value=result("[]")):
            row = demo.status()
            self.assertFalse(row["ready"])
            self.assertEqual(row["session"], "unverified")

    def test_demo_launcher_has_no_nested_lock_and_uses_absolute_local_data(self):
        demo, _ = self.make_demo()
        demo.admin.parent.mkdir(mode=0o700)
        demo.admin.write_text("SECRET")
        demo.admin.chmod(0o600)
        with patch.object(dev, "run", return_value=result()) as child:
            demo.invoke("start")
        args = child.call_args.args[0]
        self.assertEqual(args[args.index("--data-dir") + 1], str(demo.data))
        self.assertNotIn("SECRET", str(child.call_args))
        self.assertEqual(child.call_args.kwargs["env"]["MESHWAY_PERF_LOCK"], dev.LOCK)
        self.assertNotIn("pass_fds", child.call_args.kwargs)

    def test_readonly_update_refuses_before_any_docker_action(self):
        demo, meta = self.make_demo()
        meta["shell"] = "readonly"
        dev.save(demo.data / "demo.json", meta)
        with patch.object(deployment, "Docker") as docker:
            with self.assertRaisesRegex(dev.DevError, "read-only"):
                deployment.update(demo, "meshway-managedtest:new")
            docker.assert_not_called()

    def test_binary_update_preserves_containers_packages_custom_files_and_identity(self):
        demo, meta = self.make_demo()
        docker = FakeDocker(demo, meta)
        before = deployment.material(demo.data)
        custom = copy.deepcopy(docker.custom)
        with patch.object(deployment, "run", return_value=result("COMMIT\n")), redirect_stdout(io.StringIO()):
            deployment.deploy(demo, meta, "meshway-managedtest:new", docker)
        self.assertEqual(deployment.material(demo.data), before)
        self.assertEqual(docker.custom, custom)
        self.assertEqual(set(docker.containers), {"id-" + role for role in deployment.ROLES})
        self.assertEqual(set(docker.installed.values()), {docker.new})
        journal = next((self.root / ".meshway/services").glob("binary-update-*/deployment.json"))
        receipt = json.loads(journal.read_text())
        self.assertEqual(receipt["status"], "PASS")
        self.assertEqual(receipt["base_image"], meta["image_id"])
        self.assertEqual(receipt["source_image"], docker.source)
        self.assertTrue(all(record["verified"] for record in receipt["containers"].values()))
        self.assertEqual(json.loads((demo.data / "demo.json").read_text())["image_id"], meta["image_id"])
        self.assertFalse(docker.extraction)
        self.assertFalse(docker.staged)
        self.assertFalse(list(self.root.glob("meshway-dev-binary-*")))
        self.assertFalse(any(call[0] in ("restart", "stop") for call in docker.calls))

    def test_staged_digest_failure_leaves_old_binaries_and_cleans_owned_artifacts(self):
        demo, meta = self.make_demo()
        docker = FakeDocker(demo, meta)
        docker.bad_staged = True
        with patch.object(deployment, "run", return_value=result("COMMIT\n")), redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(dev.DevError, "staged binary digest"):
                deployment.deploy(demo, meta, "meshway-managedtest:new", docker)
        self.assertEqual(set(docker.installed.values()), {docker.old})
        self.assertFalse(docker.staged)
        self.assertFalse(docker.extraction)

    def test_binary_without_managed_feature_is_rejected_before_replacement(self):
        demo, meta = self.make_demo()
        docker = FakeDocker(demo, meta)
        docker.managed_feature = False
        with patch.object(deployment, "run", return_value=result("COMMIT\n")), redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(dev.DevError, "required managed runtime mode"):
                deployment.deploy(demo, meta, "meshway-managedtest:new", docker)
        self.assertEqual(set(docker.installed.values()), {docker.old})
        self.assertFalse(docker.staged)
        self.assertFalse(docker.extraction)
        self.assertFalse(any(call[0] in ("kill", "start", "restart", "stop") for call in docker.calls))

    def test_old_binary_change_during_staging_refuses_to_overwrite_it(self):
        demo, meta = self.make_demo()
        docker = FakeDocker(demo, meta)
        docker.change_current = True
        with patch.object(deployment, "run", return_value=result("COMMIT\n")), redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(dev.DevError, "current binary changed"):
                deployment.deploy(demo, meta, "meshway-managedtest:new", docker)
        self.assertEqual(docker.installed["id-nyc"], b"externally changed")
        self.assertEqual(docker.installed["id-lax"], docker.old)
        self.assertFalse(any(call[:1] == ("start",) for call in docker.calls))

    def test_container_foreign_labels_or_state_binding_are_rejected(self):
        demo, meta = self.make_demo()
        docker = FakeDocker(demo, meta)
        value = docker.inspect("id-nyc")
        value["Config"]["Labels"]["io.meshway.demo.run"] = "foreign"
        with self.assertRaisesRegex(dev.DevError, "foreign container labels"):
            deployment.ownership(value, meta, demo.data, "nyc")
        value = docker.inspect("id-nyc")
        value["Mounts"][0]["Source"] = str(self.root / "foreign")
        with self.assertRaisesRegex(dev.DevError, "state bind differs"):
            deployment.ownership(value, meta, demo.data, "nyc")

    def test_foreign_extraction_cleanup_refused(self):
        demo, meta = self.make_demo()
        docker = FakeDocker(demo, meta)
        docker.extraction = {"Image": docker.source, "State": {"Running": False}, "HostConfig": {"NetworkMode": "none"}, "Config": {"Labels": {"io.meshway.binary-update": "foreign"}}, "Id": "extraction"}
        with self.assertRaisesRegex(dev.DevError, "cleanup refused"):
            deployment.cleanup(docker, "extraction", "mine", docker.source)
        self.assertFalse(any(call[0] == "rm" for call in docker.calls))

    def test_host_artifact_cleanup_refuses_replaced_directory_or_unknown_files(self):
        directory = self.root / "artifact"
        directory.mkdir(mode=0o700)
        inode = directory.lstat().st_ino
        (directory / "foreign").write_text("keep")
        with self.assertRaisesRegex(dev.DevError, "unexpected temporary artifacts"):
            deployment.cleanup_artifact(directory, inode, None)
        self.assertTrue((directory / "foreign").exists())
        with self.assertRaisesRegex(dev.DevError, "directory changed"):
            deployment.cleanup_artifact(directory, inode + 1, None)

    def test_native_elf_validation_rejects_text_or_wrong_architecture(self):
        binary = self.root / "binary"
        for content in (b"not executable", executable(62)):
            binary.write_bytes(content)
            with self.assertRaisesRegex(dev.DevError, "native 64-bit"):
                deployment.elf(binary, "arm64")
        binary.write_bytes(executable())
        deployment.elf(binary, "arm64")

    def test_docker_children_inherit_shared_lock_descriptor(self):
        with patch.object(deployment, "run", return_value=result("ok")) as child:
            deployment.Docker(self.root, 123)("inspect", "owned")
        self.assertEqual(child.call_args.kwargs["pass_fds"], (123,))

    def test_writable_lifecycle_does_not_recreate_and_start_is_idempotent(self):
        demo, meta = self.make_demo()
        docker = FakeDocker(demo, meta)
        calls = []
        @contextmanager
        def lock():
            calls.append("enter")
            yield 123
            calls.append("exit")
        with patch.dict("sys.modules", {"performance_lock": type("Lock", (), {"performance_lock": staticmethod(lock)})}), patch.object(deployment, "Docker", return_value=docker):
            deployment.lifecycle(demo, "start")
            self.assertFalse(any(call[0] == "start" for call in docker.calls))
            deployment.lifecycle(demo, "restart")
        self.assertEqual(calls, ["enter", "exit", "enter", "exit"])
        self.assertEqual(sum(call[0] == "start" for call in docker.calls), 3)
        self.assertFalse(any(call[0] in ("create", "rm", "restart", "stop") for call in docker.calls))

    def test_docker_sigint_timeout_does_not_force_kill(self):
        demo, meta = self.make_demo()
        docker = FakeDocker(demo, meta)
        docker.ignore_sigint = True
        with patch.object(deployment.time, "monotonic", side_effect=[0, 41]):
            with self.assertRaisesRegex(dev.DevError, "graceful SIGINT deadline"):
                deployment.graceful_stop(docker, demo, meta, "nyc", "id-nyc")
        self.assertEqual([call for call in docker.calls if call[0] == "kill"], [("kill", "--signal", "SIGINT", "id-nyc")])

    def test_failed_live_command_with_ok_body_is_not_health(self):
        demo, meta = self.make_demo()
        docker = FakeDocker(demo, meta)
        docker.live_returncode = 1
        with patch.object(deployment, "run", return_value=result("COMMIT\n")), patch.object(deployment.time, "monotonic", side_effect=[0, 0, 0, 41]), redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(dev.DevError, "live CLI deadline"):
                deployment.deploy(demo, meta, "meshway-managedtest:new", docker)


class FakeDocker:
    """A stateful in-memory engine: immutable container IDs and retained writable files."""
    def __init__(self, demo, meta):
        self.demo, self.meta = demo, meta
        self.source = "sha256:" + "c" * 64
        self.old, self.new = executable(content=b"old"), executable(content=b"new")
        self.installed = {"id-" + role: self.old for role in deployment.ROLES}
        self.custom = {"id-" + role: {"apt": "dnsutils 1.2", "/usr/local/custom": "keep me"} for role in deployment.ROLES}
        self.containers = {"id-" + role: self.container(role) for role in deployment.ROLES}
        self.calls, self.staged = [], {}
        self.extraction = None
        self.bad_staged = False
        self.change_current = False
        self.ignore_sigint = False
        self.live_returncode = 0
        self.managed_feature = True

    def container(self, role):
        uid, gid = str(self.meta["uid"]), str(self.meta["gid"])
        cmd = ["--reuid", uid, "--regid", gid, "--clear-groups", "--bounding-set=-all,+net_admin", "--inh-caps=-all,+net_admin", "--ambient-caps=-all,+net_admin", "--no-new-privs", "meshway-agent", "run", "--apply-dataplane", "--tun-name", "meshway0", "--tun-mtu", "1000", "--interval-seconds", "2", "--connection-lease-seconds", "15", "--connection-profiles", "/state/profiles.json", "--state", "/state/agent.json"]
        return {"Id": "id-" + role, "Image": self.meta["image_id"], "State": {"Running": True, "StartedAt": "before"},
                "Config": {"User": "0:0", "Entrypoint": ["setpriv"], "Cmd": cmd, "StopSignal": "SIGINT", "Env": ["MESHWAY_AGENT_UID=" + uid, "MESHWAY_AGENT_GID=" + gid],
                           "Labels": {"io.meshway.demo.project": "managed-neighbors", "io.meshway.demo.run": self.meta["run_id"], "io.meshway.demo.data": self.meta["fingerprint"], "io.meshway.demo.resource": role}},
                "HostConfig": {"ReadonlyRootfs": False, "Privileged": False, "NetworkMode": "owned-network", "CapAdd": ["NET_ADMIN"], "CapDrop": [], "SecurityOpt": ["no-new-privileges"], "Devices": [{"PathOnHost": "/dev/net/tun", "PathInContainer": "/dev/net/tun", "CgroupPermissions": "rw"}]},
                "Mounts": [{"Type": "bind", "Source": str(self.demo.data / role), "Destination": "/state", "RW": True}]}

    def inspect(self, name):
        return copy.deepcopy(self.containers[name] if name in self.containers else self.extraction)

    def execute(self, *args, **kwargs):
        if args[:1] == ("exec",) and args[2:4] == ("meshway", "status"):
            return result(json.dumps({"outcome": {"status": "ok", "result": {"token": "SECRET"}}}), self.live_returncode)
        return result(self(*args, **kwargs))

    def __call__(self, *args, **kwargs):
        self.calls.append(args)
        if args[:2] == ("image", "inspect"):
            return json.dumps([{"Id": self.source, "Os": "linux", "Architecture": "arm64"}])
        if args[0] == "info":
            return "aarch64"
        if args[0] == "ps":
            role = next(arg.rsplit("=", 1)[1] for arg in args if "io.meshway.demo.resource=" in arg)
            return "id-" + role
        if args[0] == "create":
            label = args[args.index("--label") + 1].split("=", 1)[1]
            self.extraction = {"Id": "extraction", "Image": self.source, "Config": {"Labels": {"io.meshway.binary-update": label}}, "State": {"Running": False}, "HostConfig": {"NetworkMode": "none"}}
            return "extraction"
        if args[0] == "cp":
            if args[1].startswith("extraction:"):
                Path(args[2]).write_bytes(self.new)
            elif args[1].startswith("id-"):
                Path(args[2]).write_bytes(self.installed[args[1].split(":", 1)[0]])
            else:
                container, path = args[2].split(":", 1)
                self.staged[container] = Path(args[1]).read_bytes()
                if self.change_current:
                    self.installed[container] = b"externally changed"
            return ""
        if args[0] == "exec":
            container, operation = args[1], args[2]
            if operation == "dpkg-query":
                return self.custom[container]["apt"]
            if operation == "sha256sum":
                path = args[3]
                if path == "/usr/local/bin/meshway":
                    return "alias-digest  " + path
                binary = self.installed[container] if path == deployment.BIN else self.staged[container]
                sha = "wrong" if self.bad_staged and ".next-" in path else hashlib.sha256(binary).hexdigest()
                return sha + "  " + path
            if operation == "mv":
                self.installed[container] = self.staged.pop(container)
                return ""
            if operation == "rm":
                self.staged.pop(container, None)
                return ""
            if operation in ("chmod", "test"):
                return ""
            if args[-1] == "--help":
                if args[-2] == "run":
                    return "--apply-connections --apply-dataplane" if self.managed_feature else "--interval-seconds"
                return "connections stun-server"
        if args[0] == "kill":
            if not self.ignore_sigint:
                self.containers[args[-1]]["State"]["Running"] = False
            return ""
        if args[0] == "start":
            self.containers[args[1]]["State"] = {"Running": True, "StartedAt": "after"}
            return ""
        if args[0] == "rm":
            self.extraction = None
            return ""
        raise AssertionError("unexpected simulated operation: " + repr(args))


if __name__ == "__main__":
    os.umask(0o077)
    unittest.main()
