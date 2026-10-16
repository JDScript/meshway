"""Process ownership and reset transaction regressions; no live services."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import tempfile
from types import SimpleNamespace
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
        with patch.object(dev, "LocalServices", return_value=self.local), patch.object(self.local, "pane", return_value=pane), patch.object(self.local, "port", return_value=True), patch.object(self.local, "build", side_effect=lambda: order.append("build")), patch.object(self.local, "stop", side_effect=lambda service: order.append(self.local.state(service)["launchCommand"])), patch.object(self.local, "start", side_effect=lambda service, log_filter=None: order.append(dev.command(service, self.root, log_filter))), patch.object(self.local, "reset") as reset, patch.object(dev, "snapshot_datasets") as snapshot, patch("builtins.print"):
            self.assertEqual(dev.main(["update", "server"]), 0)
            self.assertEqual(dev.main(["start", "server"]), 0)
            self.assertEqual(dev.main(["update", "server", "--log-filter", "info,meshway_server=debug"]), 0)
        filtered = dev.command("server", self.root) + ["--log-filter", "info,meshway_server=debug"]
        self.assertEqual(dev.command("server", self.root, "info,meshway_server=debug"), filtered)
        self.assertEqual(order, ["build", saved["launchCommand"], dev.command("server", self.root), dev.command("server", self.root), "build", saved["launchCommand"], filtered])
        reset.assert_not_called()
        snapshot.assert_not_called()

    def test_log_filter_launch_is_recorded_owned_and_replaced_only_by_restart(self):
        launch = dev.command("server", self.root, "info,meshway_server=debug")
        running = False
        table = {101: (1, "S", " ".join(launch))}

        def run(args, **kwargs):
            nonlocal running
            if args[:2] == ["tmux", "new-session"]:
                running = True
                self.assertEqual(args[-1].split(" >>")[0], "exec " + shlex.join(launch))
                return subprocess.CompletedProcess(args, 0, "%8\t101\n", "")
            if args[:2] == ["tmux", "list-panes"]:
                text = ""
                if running:
                    saved = json.loads((self.local.directory / "server.json").read_text())
                    text = f"meshway-server\t%8\t101\t{self.root}\t{saved['startCommand']}\n"
                return subprocess.CompletedProcess(args, 0, text, "")
            self.fail("unexpected external command")

        with patch.object(dev, "run", side_effect=run), patch.object(dev, "process_table", return_value=table), patch.object(dev, "process_identity", return_value=self.identity), patch.object(self.local, "port", return_value=True), patch.object(self.local, "health", return_value=True):
            self.local.start("server", "info,meshway_server=debug")
            self.assertEqual(self.local.state("server")["launchCommand"], launch)
            self.assertEqual(self.local.pane("server")["pid"], 101)
            # Plain start keeps the running filtered launch; another filter needs restart.
            self.local.start("server")
            self.local.start("server", "info,meshway_server=debug")
            with self.assertRaisesRegex(dev.DevError, "another launch"):
                self.local.start("server", "debug")

    def test_log_filter_is_server_only_and_one_word(self):
        for argv in (["update", "docs", "--log-filter", "debug"], ["update", "all", "--log-filter", "debug"], ["stop", "server", "--log-filter", "debug"], ["status", "server", "--log-filter", "debug"]):
            with self.subTest(argv=argv), patch.object(dev, "LocalServices") as services:
                with self.assertRaisesRegex(dev.DevError, "--log-filter requires"):
                    dev.main(argv)
                services.assert_not_called()
        for value in ("info meshway_server=debug", "debug;rm", ""):
            with self.subTest(value=value), patch("sys.stderr"), self.assertRaises(SystemExit):
                dev.main(["update", "server", "--log-filter", value])
        with self.assertRaisesRegex(dev.DevError, "only to the server"):
            dev.command("docs", self.root, "debug")

    def test_build_includes_the_offline_dataset_tool_and_pinned_target(self):
        with patch.object(dev.shutil, "disk_usage", return_value=SimpleNamespace(free=50 * 1024**3)), patch.object(dev, "run") as run, patch("builtins.print"):
            self.local.build()
        args = run.call_args.args[0]
        self.assertEqual(args, ["cargo", "build", "--locked", "-p", "meshway-server", "--bin", "meshway-server", "--bin", "ip-datasets"])
        self.assertEqual(run.call_args.kwargs["env"]["CARGO_INCREMENTAL"], "0")
        self.assertEqual(run.call_args.kwargs["env"]["CARGO_TARGET_DIR"], str(self.root / "target"))

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

    def dataset_fixture(self, kinds=("geoip", "asn")):
        directory = self.root / ".meshway/ip-datasets"
        directory.mkdir(mode=0o700)
        manifest = {"version": 1, "datasets": {}}
        for kind in kinds:
            # Lifecycle fixtures only; MMDB validation belongs to the real App importer.
            content = (kind + " lifecycle fixture").encode()
            path = directory / (kind + ".mmdb")
            dev.private(path, create=True)
            path.write_bytes(content)
            manifest["datasets"][kind] = {"sha256": hashlib.sha256(content).hexdigest(), "source": "--deployment 'quoted' $(not-a-command) " + kind}
        dev.save(directory / "manifest.json", manifest)
        return directory, manifest

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

        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run") as run, patch.object(self.local, "build") as build, patch.object(self.local, "stop", side_effect=stop), patch.object(self.local, "start", side_effect=start), patch.object(self.local, "seed", side_effect=seed):
            self.local.reset()
        build.assert_not_called()
        self.assertEqual(len(run.call_args_list), 1)
        self.assertIn("--check", run.call_args.args[0])
        self.assertEqual(order, ["stop", "start", "seed"])
        self.assertEqual(self.local.reset_state()["phase"], "complete")
        journal = json.loads((self.local.directory / "server-reset.json").read_text())
        self.assertEqual(journal["datasets"], {})
        self.assertFalse((Path(journal["backup"]) / "ip-datasets").exists())
        dev.private(login)

    def test_reset_snapshots_assets_before_stop_then_archives_imports_starts_seeds(self):
        data, login = self.reset_fixture()
        assets, manifest = self.dataset_fixture()
        contents = {kind: (assets / (kind + ".mmdb")).read_bytes() for kind in manifest["datasets"]}
        inode = data.stat().st_ino
        order = []
        running = True
        rename = os.rename

        def stop(service):
            nonlocal running
            running = False
            order.append("stop")
            backup = Path(self.local.reset_state()["backup"])
            self.assertEqual(json.loads((backup / "ip-datasets/manifest.json").read_text()), manifest)
            # Deployment may publish another generation after preflight. Both imports
            # must still use this reset's already-verified files and manifest.
            for kind in manifest["datasets"]:
                (assets / (kind + ".mmdb")).write_bytes(b"next generation")
            dev.save(assets / "manifest.json", {"version": 2})

        def archive(source, destination):
            if source == data:
                order.append("archive")
            rename(source, destination)

        def run(args, **kwargs):
            if args[0] == "node":
                self.assertIn("--check", args)
                return
            self.assertFalse(running)
            kind = args[args.index("--kind") + 1]
            backup = Path(self.local.reset_state()["backup"])
            self.assertEqual(args, [str(self.root / "target/debug/ip-datasets"), "--data-dir", str(data), "import", "--kind", kind, "--file", str(backup / "ip-datasets" / (kind + ".mmdb")), "--source=" + manifest["datasets"][kind]["source"]])
            self.assertEqual(self.local.reset_state()["phase"], "datasets")
            self.assertEqual((backup / "server").stat().st_ino, inode)
            self.assertFalse(login.exists())
            self.assertEqual((backup / "ip-datasets" / (kind + ".mmdb")).read_bytes(), contents[kind])
            dev.private_tree(backup / "ip-datasets")
            if kind == "geoip":
                self.assertFalse(data.exists())
                data.mkdir(mode=0o700)
            dev.save(data / (kind + "-installed"), {"mockImport": True})
            order.append("import " + kind)

        def start(service):
            self.assertTrue((data / "geoip-installed").exists())
            self.assertTrue((data / "asn-installed").exists())
            order.append("start")

        def seed(backup):
            order.append("seed")
            dev.save(login, {"networkId": "fresh"})

        with patch.object(self.local, "pane", side_effect=lambda service: {"processes": {101}} if running else None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", side_effect=lambda path: {101} if running else set()), patch.object(self.local, "build", side_effect=lambda: order.append("build")), patch.object(dev, "run", side_effect=run), patch.object(dev.os, "rename", side_effect=archive), patch.object(self.local, "stop", side_effect=stop), patch.object(self.local, "start", side_effect=start), patch.object(self.local, "seed", side_effect=seed):
            self.local.reset()
        self.assertEqual(order, ["build", "stop", "archive", "import geoip", "import asn", "start", "seed"])
        journal = json.loads((self.local.directory / "server-reset.json").read_text())
        self.assertEqual(journal["phase"], "complete")
        self.assertEqual(journal, json.loads((Path(journal["backup"]) / "reset.json").read_text()))
        self.assertEqual(journal["datasets"], {kind: {**entry, "installed": True} for kind, entry in manifest["datasets"].items()})
        self.assertTrue(assets.exists())

    def test_reset_accepts_only_asn_and_an_already_built_importer(self):
        _, login = self.reset_fixture()
        self.dataset_fixture(("asn",))
        (self.root / "target/debug/ip-datasets").touch()
        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run") as run, patch.object(self.local, "build") as build, patch.object(self.local, "stop"), patch.object(self.local, "start"), patch.object(self.local, "seed", side_effect=lambda backup: dev.save(login, {"fresh": True})):
            self.local.reset()
        build.assert_not_called()
        imports = [call.args[0] for call in run.call_args_list if "import" in call.args[0]]
        self.assertEqual(len(imports), 1)
        self.assertEqual(imports[0][imports[0].index("--kind") + 1], "asn")

    def test_reset_rejects_invalid_dataset_manifests_before_stopping(self):
        data, login = self.reset_fixture()
        assets, valid = self.dataset_fixture()
        inode = data.stat().st_ino
        invalid = [None, [], {}, {"version": True, "datasets": valid["datasets"]},
                   {"version": 2, "datasets": valid["datasets"]}, {**valid, "extra": 1},
                   {"version": 1, "datasets": {}}, {"version": 1, "datasets": {"country": valid["datasets"]["geoip"]}}]
        entry = valid["datasets"]["geoip"]
        entries = [None, {}, {**entry, "file": "elsewhere.mmdb"}, {**entry, "sha256": "A" * 64},
                   {**entry, "sha256": "0" * 63}, {**entry, "sha256": 1}]
        entries += [{**entry, "source": value} for value in (None, "", "x" * 257, "国" * 86, "x\n", "x\0", "x\x7f", "x\x85", "\ud800")]
        invalid += [{"version": 1, "datasets": {"geoip": item}} for item in entries]
        texts = [json.dumps(item) for item in invalid] + ["{", '{"version":1,"version":1,"datasets":{}}', json.dumps(valid).replace('"source":', '"source":"duplicate","source":', 1), json.dumps(valid) + " " * 8192]
        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run"), patch.object(self.local, "build") as build, patch.object(self.local, "stop") as stop, patch.object(self.local, "start") as start:
            for text in texts:
                with self.subTest(manifest=text):
                    (assets / "manifest.json").write_text(text)
                    with self.assertRaises(dev.DevError):
                        self.local.reset()
                    self.assertEqual(data.stat().st_ino, inode)
                    self.assertTrue(login.exists())
            build.assert_not_called()
            stop.assert_not_called()
            start.assert_not_called()

    def test_reset_rejects_missing_empty_oversize_and_hash_mismatched_assets_before_stop(self):
        data, login = self.reset_fixture()
        assets, _ = self.dataset_fixture()
        path = assets / "geoip.mmdb"
        content = path.read_bytes()
        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run"), patch.object(self.local, "stop") as stop:
            for kind in ("missing-manifest", "missing-file", "empty", "oversize", "hash"):
                with self.subTest(kind=kind):
                    manifest = (assets / "manifest.json").read_bytes()
                    if kind == "missing-manifest":
                        (assets / "manifest.json").unlink()
                    elif kind == "missing-file":
                        path.unlink()
                    else:
                        with path.open("wb") as output:
                            if kind == "oversize":
                                output.truncate(dev.MAX_DATASET_BYTES + 1)
                            elif kind == "hash":
                                output.write(b"mismatched bytes")
                    with self.assertRaises(dev.DevError):
                        self.local.reset()
                    self.assertTrue(data.exists())
                    self.assertTrue(login.exists())
                    dev.private(assets / "manifest.json", create=True)
                    (assets / "manifest.json").write_bytes(manifest)
                    dev.private(path, create=True)
                    path.write_bytes(content)
            stop.assert_not_called()

    def test_reset_rejects_dataset_permissions_links_and_non_regular_files_before_stop(self):
        self.reset_fixture()
        assets, _ = self.dataset_fixture()
        targets = [assets, assets / "manifest.json", assets / "geoip.mmdb"]
        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run"), patch.object(self.local, "stop") as stop:
            for target in targets:
                for kind in ("mode", "symlink", "hardlink", "fifo"):
                    if target == assets and kind in ("hardlink", "fifo"):
                        continue
                    with self.subTest(path=target.name, kind=kind):
                        other = target.with_name(target.name + "-original")
                        if kind == "mode":
                            target.chmod(0o755 if target == assets else 0o644)
                        elif kind == "hardlink":
                            os.link(target, other)
                        else:
                            target.rename(other)
                            if kind == "symlink":
                                target.symlink_to(other)
                            else:
                                os.mkfifo(target, 0o600)
                        with self.assertRaises(dev.DevError):
                            self.local.reset()
                        if kind == "mode":
                            target.chmod(0o700 if target == assets else 0o600)
                        elif kind == "hardlink":
                            other.unlink()
                        else:
                            target.unlink()
                            other.rename(target)
            stop.assert_not_called()

    def test_failed_import_preserves_original_and_partial_databases_without_start_or_retry(self):
        data, login = self.reset_fixture()
        self.dataset_fixture()
        imports = []

        def run(args, **kwargs):
            if args[0] == "node":
                return
            kind = args[args.index("--kind") + 1]
            imports.append(kind)
            if kind == "asn":
                raise dev.DevError("SECRET")
            data.mkdir(mode=0o700)
            dev.save(data / "partial-db", {"geoipImported": True})

        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(self.local, "build"), patch.object(dev, "run", side_effect=run), patch.object(self.local, "stop"), patch.object(self.local, "start") as start, patch.object(self.local, "seed") as seed:
            with self.assertRaisesRegex(dev.DevError, "reset failed during datasets") as failure:
                self.local.reset()
            start.assert_not_called()
            seed.assert_not_called()
        self.assertNotIn("SECRET", str(failure.exception))
        self.assertEqual(imports, ["geoip", "asn"])
        journal = json.loads((self.local.directory / "server-reset.json").read_text())
        self.assertEqual((journal["phase"], journal["failedAt"]), ("failed", "datasets"))
        self.assertTrue(journal["datasets"]["geoip"]["installed"])
        self.assertFalse(journal["datasets"]["asn"]["installed"])
        backup = Path(journal["backup"])
        self.assertTrue((backup / "server/controller.sqlite").exists())
        self.assertTrue((backup / "dev-login.json").exists())
        dev.private_tree(backup / "ip-datasets")
        self.assertTrue((data / "partial-db").exists())
        self.assertFalse(login.exists())

    def test_reset_rejects_foreign_uid_on_dataset_directory_manifest_or_file_before_stop(self):
        self.reset_fixture()
        assets, _ = self.dataset_fixture()
        lstat = Path.lstat

        def foreign_uid(path, *args, **kwargs):
            info = lstat(path, *args, **kwargs)
            if path == target:
                fields = list(info)
                fields[4] += 1
                return os.stat_result(fields)
            return info

        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(dev, "run"), patch.object(self.local, "stop") as stop:
            for target in (assets, assets / "manifest.json", assets / "geoip.mmdb"):
                with self.subTest(path=target.name), patch.object(Path, "lstat", new=foreign_uid):
                    with self.assertRaisesRegex(dev.DevError, "ownership or permissions"):
                        self.local.reset()
            stop.assert_not_called()

    def test_reset_refuses_import_when_server_reappears_after_archive(self):
        self.reset_fixture()
        self.dataset_fixture()
        with patch.object(self.local, "pane", return_value={"processes": {101}}), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", return_value=set()), patch.object(self.local, "build"), patch.object(dev, "run") as run, patch.object(self.local, "stop"), patch.object(self.local, "start") as start:
            with self.assertRaisesRegex(dev.DevError, "reset failed during datasets"):
                self.local.reset()
            start.assert_not_called()
        self.assertEqual(len(run.call_args_list), 1)
        self.assertIn("--check", run.call_args.args[0])

    def test_reset_refuses_next_import_when_fresh_data_directory_becomes_active(self):
        data, _ = self.reset_fixture()
        self.dataset_fixture()
        imports = []

        def run(args, **kwargs):
            if args[0] != "node":
                imports.append(args[args.index("--kind") + 1])
                data.mkdir(mode=0o700)
                dev.save(data / "partial-db", {"geoipImported": True})

        with patch.object(self.local, "pane", return_value=None), patch.object(self.local, "port", return_value=False), patch.object(self.local, "data_users", side_effect=lambda path: {777} if imports else set()), patch.object(self.local, "build"), patch.object(dev, "run", side_effect=run), patch.object(self.local, "stop"), patch.object(self.local, "start") as start:
            with self.assertRaisesRegex(dev.DevError, "reset failed during datasets"):
                self.local.reset()
            start.assert_not_called()
        self.assertEqual(imports, ["geoip"])
        self.assertTrue((data / "partial-db").exists())

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
