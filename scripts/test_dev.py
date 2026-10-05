"""Process-ownership regression tests without live service mutations."""
import json
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
        self.local = dev.LocalServices(self.root, wait=0)

    def tearDown(self):
        self.temp.cleanup()

    def test_ownership_matches_command_path_and_process_before_adoption(self):
        command = dev.command("server", self.root)
        log = self.local.directory / "meshway-server.log"
        shell = "exec " + shlex.join(command) + " >>" + shlex.quote(str(log)) + " 2>&1"
        output = f"meshway-server\t%8\t101\t{self.root}\t{shell}\n"
        table = {101: (1, "S", " ".join(command)), 102: (101, "S", "worker")}
        with patch.object(dev, "run", return_value=subprocess.CompletedProcess([], 0, output, "")), patch.object(dev, "process_table", return_value=table):
            self.assertEqual(self.local.pane("server")["processes"], {101, 102})
            table[101] = (1, "S", "foreign process")
            with self.assertRaises(dev.DevError):
                self.local.pane("server")
        self.assertFalse(dev.known_start(shell + " ; sleep 1", "server", self.root, log))
        self.assertFalse(dev.known_start(shell.replace(shlex.quote(str(log)), shlex.quote(str(self.root / "other.log"))), "server", self.root, log))
        self.assertIn("--console-origin", command)
        self.assertEqual(command[command.index("--console-origin") + 1], "http://127.0.0.1:3001")
        self.assertFalse(dev.known_start(shell.replace(dev.CONSOLE_ORIGIN, "http://127.0.0.1:3002"), "server", self.root, log))

    def test_foreign_listener_never_starts_or_stops_a_service(self):
        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "listeners", return_value={777}), patch.object(dev, "run") as run, patch.object(dev.os, "kill") as kill:
            for operation in [self.local.start, self.local.stop]:
                with self.assertRaisesRegex(dev.DevError, "foreign listener"):
                    operation("server")
            run.assert_not_called()
            kill.assert_not_called()

    def test_start_is_idempotent_and_persists_only_its_identity(self):
        pane = {"pane": "%8", "pid": 101, "processes": {101}}
        with patch.object(self.local, "pane", return_value=pane), patch.object(self.local, "port", return_value=True), patch.object(self.local, "health", return_value=True), patch.object(dev, "run") as run:
            self.local.start("server")
            self.local.start("server")
            run.assert_not_called()
        state = self.local.directory / "server.json"
        self.assertEqual(json.loads(state.read_text())["root"], str(self.root))
        dev.private(state)

    def test_stop_only_signals_verified_tree_and_has_no_force_fallback(self):
        pane = {"pane": "%8", "pid": 101, "processes": {101, 102}}
        table = {101: (1, "S", "server"), 102: (101, "S", "worker"), 777: (1, "S", "foreign")}
        with patch.object(self.local, "pane", return_value=pane), patch.object(self.local, "port", return_value=True), patch.object(dev, "process_table", return_value=table), patch.object(dev.os, "kill") as kill:
            with self.assertRaisesRegex(dev.DevError, "SIGINT timeout"):
                self.local.stop("server")
            self.assertEqual({call.args for call in kill.call_args_list}, {(101, signal.SIGINT), (102, signal.SIGINT)})

    def test_status_does_not_expose_exception_payload(self):
        with patch.object(self.local, "pane", side_effect=dev.DevError("SECRET")):
            result = self.local.status("server")
        self.assertNotIn("SECRET", json.dumps(result))
        self.assertFalse(result["ready"])

    def test_docs_start_command_remains_recognizable(self):
        log = self.local.directory / "meshway-docs.log"
        command = "exec pnpm --filter @meshway/docs dev >>" + shlex.quote(str(log)) + " 2>&1"
        self.assertTrue(dev.known_start(command, "docs", self.root, log))
        self.assertTrue(dev.actual_command("node /usr/local/bin/pnpm --filter @meshway/docs dev", "docs", self.root))
        self.assertFalse(dev.known_start(command.replace("exec pnpm", "exec env TOKEN=SECRET pnpm"), "docs", self.root, log))


if __name__ == "__main__":
    unittest.main()
