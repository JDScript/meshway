"""Process ownership and reset transaction regressions; no live services."""
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import dev


class Services(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="meshway dev 'quoted' ")
        self.root = Path(self.temp.name).resolve()
        (self.root / ".meshway").mkdir(mode=0o700)
        (self.root / ".meshway/services").mkdir(mode=0o700)
        (self.root / "target/debug").mkdir(parents=True)
        (self.root / "target/debug/meshway-server").touch()
        self.local = dev.LocalServices(self.root, wait=0)
        self.identity = "Mon Oct  5 12:00:00 2026"

    def tearDown(self):
        self.temp.cleanup()

    def record(self, service="server", launch=None):
        launch = launch or dev.command(service, self.root)
        log = self.local.directory / ("meshway-" + service + ".log")
        shell = "exec " + shlex.join(launch) + " >>" + shlex.quote(str(log)) + " 2>&1"
        saved = {"version": 2, "root": str(self.root), "service": service,
                 "pane": "%8", "pid": 101, "port": dev.SERVICES[service][0],
                 "launchCommand": launch, "startCommand": shell,
                 "processCommand": " ".join(launch), "processIdentity": self.identity}
        dev.save(self.local.directory / (service + ".json"), saved)
        return saved

    def test_old_launch_identity_survives_new_desired_command(self):
        old = dev.command("server", self.root)[:-2]
        saved = self.record(launch=old)
        output = f"meshway-server\t%8\t101\t{self.root}\t{saved['startCommand']}\n"
        table = {101: (1, "S", " ".join(old)), 102: (101, "S", "worker")}
        with patch.object(dev, "run", return_value=subprocess.CompletedProcess([], 0, output, "")), patch.object(dev, "process_table", return_value=table), patch.object(dev, "process_identity", return_value=self.identity):
            self.assertEqual(self.local.pane("server")["processes"], {101, 102})
            # Editing desired CLI neither adopts nor invalidates the prior launch.
            with patch.object(dev, "command", return_value=["next", "--different"]):
                self.assertEqual(self.local.pane("server")["pid"], 101)
            table[101] = (1, "S", "foreign process")
            with self.assertRaises(dev.DevError):
                self.local.pane("server")

    def test_unrecorded_matching_session_is_not_adopted(self):
        saved = self.record()
        (self.local.directory / "server.json").unlink()
        output = f"meshway-server\t%8\t101\t{self.root}\t{saved['startCommand']}\n"
        with patch.object(dev, "run", return_value=subprocess.CompletedProcess([], 0, output, "")):
            with self.assertRaisesRegex(dev.DevError, "missing launch identity"):
                self.local.pane("server")

    def test_rejects_forged_pid_pane_command_cwd_and_reused_pid(self):
        saved = self.record()
        valid = ["meshway-server", "%8", "101", str(self.root), saved["startCommand"]]
        table = {101: (1, "S", saved["processCommand"])}
        for index, value in [(1, "%9"), (2, "777"), (3, "/foreign"), (4, ""), (4, saved["startCommand"] + " ; sleep 1")]:
            fields = valid.copy()
            fields[index] = value
            with self.subTest(index=index, value=value), patch.object(dev, "run", return_value=subprocess.CompletedProcess([], 0, "\t".join(fields), "")), patch.object(dev, "process_table", return_value=table), patch.object(dev, "process_identity", return_value=self.identity):
                with self.assertRaises(dev.DevError):
                    self.local.pane("server")
        with patch.object(dev, "run", return_value=subprocess.CompletedProcess([], 0, "\t".join(valid), "")), patch.object(dev, "process_table", return_value=table), patch.object(dev, "process_identity", return_value="another start"):
            with self.assertRaisesRegex(dev.DevError, "identity differs"):
                self.local.pane("server")

    def test_foreign_listener_never_starts_stops_or_resets(self):
        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "listeners", return_value={777}), patch.object(dev, "run") as run, patch.object(dev.os, "kill") as kill:
            for operation in [lambda: self.local.start("server"), lambda: self.local.stop("server"), self.local.reset]:
                with self.assertRaisesRegex(dev.DevError, "foreign listener"):
                    operation()
            run.assert_not_called()
            kill.assert_not_called()

    def test_start_records_the_tmux_launch_before_readiness(self):
        running = False
        table = {101: (1, "S", " ".join(dev.command("server", self.root)))}

        def run(args, **kwargs):
            nonlocal running
            if args[:2] == ["tmux", "new-session"]:
                running = True
                return subprocess.CompletedProcess(args, 0, "%8\t101\n", "")
            if args[:2] == ["tmux", "list-panes"]:
                text = ""
                if running:
                    saved = json.loads((self.local.directory / "server.json").read_text())
                    text = f"meshway-server\t%8\t101\t{self.root}\t{saved['startCommand']}\n"
                return subprocess.CompletedProcess(args, 0, text, "")
            self.fail("unexpected external command")

        with patch.object(dev, "run", side_effect=run) as runner, patch.object(dev, "process_table", return_value=table), patch.object(dev, "process_identity", return_value=self.identity), patch.object(self.local, "port", return_value=True), patch.object(self.local, "health", return_value=True):
            self.local.start("server")
            self.local.start("server")
        state = self.local.state("server")
        self.assertEqual(state["launchCommand"], dev.command("server", self.root))
        self.assertEqual(state["processCommand"], table[101][2])
        self.assertEqual(sum(call.args[0][:2] == ["tmux", "new-session"] for call in runner.call_args_list), 1)
        dev.private(self.local.directory / "server.json")

    def test_update_stops_old_record_then_launches_current_command_without_reset(self):
        saved = self.record(launch=dev.command("server", self.root)[:-2])
        order = []
        pane = {"pane": "%8", "pid": 101, "processes": {101}}
        with patch.object(dev, "LocalServices", return_value=self.local), patch.object(self.local, "pane", return_value=pane), patch.object(self.local, "port", return_value=True), patch.object(self.local, "build", side_effect=lambda: order.append("build")), patch.object(self.local, "stop", side_effect=lambda service: order.append(self.local.state(service)["launchCommand"])), patch.object(self.local, "start", side_effect=lambda service: order.append(dev.command(service, self.root))), patch.object(self.local, "reset") as reset, patch("builtins.print"):
            self.assertEqual(dev.main(["update", "server"]), 0)
        self.assertEqual(order, ["build", saved["launchCommand"], dev.command("server", self.root)])
        reset.assert_not_called()

    def test_stop_only_signals_verified_tree_and_has_no_force_fallback(self):
        pane = {"pane": "%8", "pid": 101, "processes": {101, 102}}
        table = {101: (1, "S", "server"), 102: (101, "S", "worker"), 777: (1, "S", "foreign")}
        with patch.object(self.local, "pane", return_value=pane), patch.object(self.local, "port", return_value=True), patch.object(dev, "process_table", return_value=table), patch.object(dev, "process_identity", return_value=self.identity), patch.object(dev.os, "kill") as kill:
            with self.assertRaisesRegex(dev.DevError, "SIGINT timeout"):
                self.local.stop("server")
            self.assertEqual({call.args for call in kill.call_args_list}, {(101, signal.SIGINT), (102, signal.SIGINT)})

    def test_status_does_not_expose_exception_payload(self):
        with patch.object(self.local, "pane", side_effect=dev.DevError("SECRET")):
            result = self.local.status("server")
        self.assertNotIn("SECRET", json.dumps(result))
        self.assertFalse(result["ready"])

    def test_invalid_reset_journal_cannot_report_ready(self):
        dev.save(self.local.directory / "server-reset.json", {"phase": "complete"})
        with patch.object(self.local, "pane", return_value={"pid": 101}), patch.object(self.local, "port", return_value=True), patch.object(self.local, "health", return_value=True):
            result = self.local.status("server")
        self.assertFalse(result["ready"])
        self.assertEqual(result["session"], "unverified")

    def test_docs_command_matches_only_recorded_launcher(self):
        log = self.local.directory / "meshway-docs.log"
        launch = dev.command("docs", self.root)
        command = "exec pnpm --filter @meshway/docs dev >>" + shlex.quote(str(log)) + " 2>&1"
        self.assertTrue(dev.known_start(command, launch, self.root, log))
        self.assertTrue(dev.actual_command("node /usr/local/bin/pnpm --filter @meshway/docs dev", "docs", launch))
        self.assertFalse(dev.known_start(command.replace("exec pnpm", "exec env TOKEN=SECRET pnpm"), launch, self.root, log))

    def reset_fixture(self):
        data = self.root / ".meshway/server"
        data.mkdir(mode=0o700)
        for name in ("controller.lock", "controller.sqlite", "controller.sqlite-wal", "private-key"):
            dev.save(data / name, {"marker": name})
        login = self.root / ".meshway/dev-login.json"
        dev.save(login, {"password": "private test marker"})
        return data, login

    def test_reset_stops_archives_whole_private_directory_then_starts_seeds(self):
        data, login = self.reset_fixture()
        inode = data.stat().st_ino
        previous = login.read_bytes()
        order = []

        def stop(service):
            self.assertTrue(data.exists())
            self.assertTrue(login.exists())
            order.append("stop")

        def start(service):
            self.assertFalse(data.exists())
            self.assertFalse(login.exists())
            order.append("start")
            data.mkdir(mode=0o700)

        def seed(backup):
            self.assertEqual((backup / "server").stat().st_ino, inode)
            self.assertEqual((backup / "dev-login.json").read_bytes(), previous)
            self.assertEqual(len(list((backup / "server").iterdir())), 4)
            dev.private(backup, directory=True)
            self.assertEqual(self.local.reset_state()["phase"], "seeding")
            order.append("seed")
            dev.save(login, {"networkId": "new-network"})

        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run"), patch.object(self.local, "stop", side_effect=stop), patch.object(self.local, "start", side_effect=start), patch.object(self.local, "seed", side_effect=seed):
            self.local.reset()
        self.assertEqual(order, ["stop", "start", "seed"])
        self.assertEqual(self.local.reset_state()["phase"], "complete")
        dev.private(login)

    def test_failed_seed_keeps_original_backup_and_partial_database_without_retry(self):
        data, login = self.reset_fixture()

        def start(service):
            data.mkdir(mode=0o700)
            dev.save(data / "partial-db", {"partial": True})

        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run"), patch.object(self.local, "stop"), patch.object(self.local, "start", side_effect=start) as started, patch.object(self.local, "seed", side_effect=dev.DevError("SECRET")) as seeded:
            with self.assertRaisesRegex(dev.DevError, "reset failed during seeding") as failure:
                self.local.reset()
        self.assertNotIn("SECRET", str(failure.exception))
        started.assert_called_once()
        seeded.assert_called_once()
        state = self.local.reset_state()
        self.assertEqual(state["phase"], "failed")
        self.assertTrue((Path(state["backup"]) / "server/controller.sqlite").exists())
        self.assertTrue((Path(state["backup"]) / "dev-login.json").exists())
        self.assertTrue((data / "partial-db").exists())
        self.assertFalse(login.exists())
        with patch.object(self.local, "pane", return_value={"pid": 101}), patch.object(self.local, "port", return_value=True), patch.object(self.local, "health", return_value=True):
            self.assertFalse(self.local.status("server")["ready"])

    def test_reset_rejects_foreign_data_user_before_stop(self):
        self.reset_fixture()
        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value={777}), patch.object(self.local, "stop") as stop:
            with self.assertRaisesRegex(dev.DevError, "foreign process"):
                self.local.reset()
            stop.assert_not_called()

    def test_stop_failure_preserves_original_data_and_prevents_archive_start_seed(self):
        data, login = self.reset_fixture()
        inode = data.stat().st_ino
        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run"), patch.object(self.local, "stop", side_effect=dev.DevError("SIGINT timeout")), patch.object(self.local, "start") as start, patch.object(self.local, "seed") as seed:
            with self.assertRaisesRegex(dev.DevError, "reset failed during stopping"):
                self.local.reset()
            start.assert_not_called()
            seed.assert_not_called()
        self.assertEqual(data.stat().st_ino, inode)
        self.assertTrue(login.exists())
        self.assertEqual(self.local.reset_state()["phase"], "failed")

    def test_reset_refuses_held_database_lock_after_stop(self):
        data, login = self.reset_fixture()
        with (data / "controller.lock").open("r+") as lock:
            dev.fcntl.flock(lock.fileno(), dev.fcntl.LOCK_EX | dev.fcntl.LOCK_NB)
            with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run"), patch.object(self.local, "stop"), patch.object(self.local, "start") as start:
                with self.assertRaisesRegex(dev.DevError, "reset failed during archiving"):
                    self.local.reset()
                start.assert_not_called()
        self.assertTrue((data / "controller.sqlite").exists())
        self.assertTrue(login.exists())

    def test_reset_rejects_non_private_tree_symlinks_and_hard_links(self):
        data, _ = self.reset_fixture()
        target = data / "private-key"
        for kind in ("directory", "file", "symlink", "hardlink"):
            with self.subTest(kind=kind):
                if kind == "directory":
                    data.chmod(0o755)
                elif kind == "file":
                    target.chmod(0o644)
                elif kind == "symlink":
                    (data / "foreign").symlink_to(target)
                else:
                    os.link(target, data / "foreign")
                with patch.object(self.local, "stop") as stop, patch.object(dev, "run") as run:
                    with self.assertRaises(dev.DevError):
                        self.local.reset()
                    stop.assert_not_called()
                    run.assert_not_called()
                data.chmod(0o700)
                target.chmod(0o600)
                (data / "foreign").unlink(missing_ok=True)

    def test_reset_requires_explicit_server_selection(self):
        for args in (["reset"], ["reset", "all"], ["reset", "docs"]):
            with self.assertRaisesRegex(dev.DevError, "explicit server"):
                dev.main(args)


if __name__ == "__main__":
    unittest.main()
