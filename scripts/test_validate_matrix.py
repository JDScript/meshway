"""Matrix orchestration regressions; no real builds, Docker, SSH or measurements."""
import json
import contextlib
import os
from pathlib import Path
import subprocess
import tempfile
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import mock_open, patch

import validate_matrix as matrix
import validate_remote as remote
import validation_support as support
import validation_progress as progress
from validation_support import import_artifact


class MatrixTests(unittest.TestCase):
    def setUp(self):
        environment = patch.dict(os.environ)
        environment.start()
        self.addCleanup(environment.stop)
        os.environ.pop("MESHWAY_VALIDATION_ROOT", None)
        os.environ.pop("MESHWAY_VALIDATION_TARGET", None)

    def test_failure_summary_keeps_private_error_details_out_of_stdout(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary)
            (report / "error.txt").write_text("credential=must-stay-private")
            (report / "a.log").write_text(json.dumps({"fields": {"phase": "path_handshake", "detail": "must-stay-private"}}))
            excerpt = matrix.failure_excerpt(report, report / "missing-invocation.log")
            self.assertIn("error.txt", excerpt)
            self.assertIn("path_handshake", excerpt)
            self.assertNotIn("must-stay-private", excerpt)

    def test_remote_progress_reports_live_phase_and_completed_rows_without_private_details(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            identity = "20261008T010000Z-progress"
            directory = root / "matrix" / identity
            report = directory / "rows/nat-off/report"
            report.mkdir(parents=True)
            (directory / "matrix.json").write_text(json.dumps({
                "status": "running", "selectedRows": ["nat/off", "tcp/off", "udp/off"],
                "rows": [{"row": "tcp/off", "passed": True, "failureExcerpt": "secret"}],
            }))
            (report / "a.log").write_text(json.dumps({"timestamp": "2026-10-08T01:01:00Z", "fields": {
                "phase": "persistent-traffic", "detail": "secret", "message": "secret",
            }}) + "\n")
            (report / "error.txt").write_text("secret")
            state = progress.snapshot(root, identity)["progress"]
            rendered = progress.display(state)
            self.assertIn("nat/off=a:persistent-traffic", rendered)
            self.assertIn("tcp/off=passed", rendered)
            self.assertIn("udp/off=queued", rendered)
            self.assertNotIn("secret", rendered)

    def test_completed_remote_progress_keeps_the_recorded_duration_on_late_attach(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            identity = "20261008T010000Z-completed"
            directory = root / "matrix" / identity
            directory.mkdir(parents=True)
            (directory / "matrix.json").write_text(json.dumps({
                "status": "complete", "elapsedSeconds": 64.8, "selectedRows": [], "rows": [],
            }))
            state = progress.snapshot(root, identity)["progress"]
            self.assertEqual(state["elapsedSeconds"], 64)
            self.assertIn("1m04s", progress.display(state))

    def test_imported_binaries_share_only_frozen_outputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source = directory / "built"
            source.write_bytes(b"frozen fixture")
            with self.assertRaisesRegex(RuntimeError, "frozen"):
                support.link_artifact(source, directory / "mutable")
            source.chmod(0o500)
            destination = directory / "imported"
            support.link_artifact(source, destination)
            self.assertEqual(source.stat().st_ino, destination.stat().st_ino)
            self.assertEqual(destination.read_bytes(), b"frozen fixture")

    def test_rootful_evidence_export_is_labelled_private_and_exact(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary)
            marker = report / "signals/ready.json"
            calls = []
            def run(*command, **kwargs):
                calls.append(command)
                return "owned-helper"
            with patch.object(support, "rootful_bind_access", return_value=True), patch.object(support, "run", side_effect=run):
                support.export_evidence(report, "pinned-image", "linux/amd64", "runtime", "run", [marker])
            creation = calls[0]
            self.assertIn("io.meshway.runtime-fixture=run", creation)
            self.assertIn("type=bind,source=" + str(report) + ",target=/evidence", creation)
            self.assertEqual(creation[-1], "/evidence/signals/ready.json")
            self.assertIn("-hR", creation)
            self.assertNotIn("--privileged", creation)
            self.assertEqual(calls[-1], ("docker", "rm", "-f", "owned-helper"))

    def test_native_image_reuses_perf_lookup_then_runtime_provenance(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = matrix.parse_args(["--rows", "udp/off"])
            for platform in ("linux/amd64", "linux/arm64"):
                with patch.object(matrix, "ROOT", root), patch.object(matrix, "build_lock", return_value=contextlib.nullcontext()), \
                        patch.object(matrix, "image_builder", return_value=SimpleNamespace(source_digest=lambda: "locked")), \
                        patch.object(matrix, "image_for", side_effect=lambda platform, _: {"imageId": "sha256:existing", "sourceDigest": "locked", "status": "built", "platform": platform}) as lookup, \
                        patch.object(matrix, "base_image", return_value="pinned"), patch.object(matrix, "verify_image") as verify:
                    images = matrix.prepare_images(args, root, platform)
                self.assertEqual(lookup.call_args.args[0], platform)
                self.assertEqual(verify.call_args.args[0]["imageId"], "sha256:existing")
                self.assertEqual(json.loads(images["runtime"].read_text())["platform"], platform)

    def test_an_earlier_partial_manifest_is_replaced_by_the_full_shape(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = matrix.parse_args(["--rows", "udp/off"])
            full = {"imageId": "sha256:existing", "sourceDigest": "locked", "status": "built",
                    "platform": "linux/arm64", "packages": [{"name": "iproute2"}]}
            def verify(data, platform):
                if not data.get("packages"):
                    raise matrix.DevError("partial")
            with patch.object(matrix, "ROOT", root), patch.object(matrix, "validation_root", return_value=root), \
                    patch.object(matrix, "build_lock", return_value=contextlib.nullcontext()), \
                    patch.object(matrix, "image_builder", return_value=SimpleNamespace(source_digest=lambda: "locked")), \
                    patch.object(matrix, "image_for", return_value=full) as lookup, \
                    patch.object(matrix, "verify_image", side_effect=verify), \
                    patch.object(matrix.subprocess, "run", return_value=SimpleNamespace(returncode=0)):
                manifest = root / "os" / "runtime-arm64-locked" / "image-manifest.json"
                manifest.parent.mkdir(parents=True)
                manifest.write_text(json.dumps({"imageId": "sha256:existing", "status": "built", "platform": "linux/arm64"}))
                images = matrix.prepare_images(args, root, "linux/arm64")
            lookup.assert_called_once()
            self.assertEqual(json.loads(images["runtime"].read_text())["packages"], [{"name": "iproute2"}])

    def test_default_parallel_is_a_quarter_of_the_cpus_bounded_by_memory_and_rows(self):
        plenty = mock_open(read_data="MemTotal: 1 kB\nMemAvailable: 62914560 kB\n")
        with patch.object(matrix.os, "sched_getaffinity", return_value=set(range(32)), create=True), patch("builtins.open", plenty):
            self.assertEqual(matrix.parse_args([]).parallel, 8)
            self.assertEqual(matrix.parse_args(["--rows", "tun", "packet"]).parallel, 2)
            self.assertEqual(matrix.parse_args(["--parallel", "3"]).parallel, 3)
        with patch.object(matrix.os, "sched_getaffinity", return_value={0}, create=True), patch("builtins.open", plenty):
            self.assertEqual(matrix.parse_args([]).parallel, 1)
        # A shared host with 10 GiB available only gets three rows at a time.
        scarce = mock_open(read_data="MemAvailable: 10485760 kB\n")
        with patch.object(matrix.os, "sched_getaffinity", return_value=set(range(32)), create=True), patch("builtins.open", scarce):
            self.assertEqual(matrix.parse_args([]).parallel, 3)
        # Without /proc/meminfo the CPU bound stands alone.
        with patch.object(matrix.os, "sched_getaffinity", return_value=set(range(12)), create=True), patch("builtins.open", side_effect=OSError):
            self.assertEqual(matrix.parse_args([]).parallel, 3)

    def test_build_lock_lives_in_the_overridable_shared_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.assertEqual(support.cargo_target(), support.ROOT / "target/node-kernel-tests")
            os.environ["MESHWAY_VALIDATION_TARGET"] = temporary
            self.assertEqual(support.cargo_target(), Path(temporary))
            with support.build_lock():
                self.assertTrue((Path(temporary) / "build.lock").exists())

    def test_rows_are_twenty_two_and_filter_keeps_order_without_duplicates(self):
        self.assertEqual(len(matrix.DEFAULT_ROWS), 22)
        self.assertEqual(len(matrix.ROWS), 22)
        self.assertEqual([name for name in matrix.DEFAULT_ROWS if name.startswith("roam/")],
                         ["roam/direct", "roam/nat", "roam/relay", "roam/flap"])
        self.assertIn("nat/relay", matrix.DEFAULT_ROWS)
        self.assertIn("nat/reconnect", matrix.DEFAULT_ROWS)
        self.assertEqual([name for name in matrix.DEFAULT_ROWS if name.startswith("netmap/")],
                         ["netmap/offline", "netmap/revoke", "netmap/expiry", "netmap/restart"])
        self.assertIn("exit/off", matrix.parse_args([]).rows)
        self.assertEqual(matrix.parse_args(["--rows", "exit/off"]).rows, ["exit/off"])
        self.assertEqual(matrix.parse_args(["--rows", "nat/off", "nat+fallback/off", "nat/off"]).rows,
                         ["nat/off", "nat+fallback/off"])
        with patch("sys.stderr"), self.assertRaises(SystemExit):
            matrix.parse_args(["--parallel", "0"])
        with patch("sys.stderr"), self.assertRaises(SystemExit):
            matrix.parse_args(["--run", "../other"])

    def test_packet_nested_daemon_and_all_runtime_rows_reuse_artifacts(self):
        artifacts = {kind: Path("/artifacts") / kind for kind in ("runtime", "packet", "tun")}
        images = {kind: Path("/images") / kind for kind in ("runtime", "dns", "traversal")}
        for name in matrix.ROWS:
            command = matrix.row_command(name, Path("/report"), artifacts, images, True)
            self.assertIn("--artifacts", command)
            self.assertNotIn("--build-only", command)
            self.assertEqual("--dns-image-manifest" in command, name.endswith(("/resolved", "/resolv-conf")))
            self.assertEqual("--traversal-image-manifest" in command, name.startswith(("nat", "roam")))
            self.assertEqual("--short" in command, name != "tun")
            if name == "packet":
                self.assertIn("--runtime-artifacts", command)

    def test_streaming_rows_select_only_the_requested_carrier_and_reject_nat(self):
        for carrier in ("vless", "trojan"):
            command = matrix.row_command(carrier + "/off", Path("/report"),
                                        {"runtime": Path("/artifacts")},
                                        {"runtime": Path("/image")}, False)
            self.assertEqual(command[command.index("--transport") + 1], carrier)
            self.assertNotIn("--traversal", command)
            self.assertNotIn("--short", command)
            result = subprocess.run([sys.executable, str(matrix.ROOT / "scripts/verify_runtime.py"),
                                     "--transport", carrier, "--traversal", "nat"],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn("--traversal nat requires --transport native-udp", result.stderr)

    def test_failure_does_not_cancel_other_rows_and_builds_once(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            builds = []
            def invoke(command, logfile):
                builds.append(command)
                return 0
            def execute(name, directory, artifacts, images, args):
                return dict(row=name, passed=name != "nat/off", elapsedSeconds=1,
                            loadBeforeStart=dict(loadAverage=[0, 0, 0]), scope="diagnostic-short")
            with patch.object(matrix, "ROOT", root), patch.object(matrix, "load", return_value=dict(uptime="fixture uptime", loadAverage=[0, 0, 0])), \
                    patch.object(matrix, "docker_target", return_value=("target", "linux/amd64", 62)), \
                    patch.object(matrix, "prepare_images", return_value={}), patch.object(matrix, "invoke", side_effect=invoke), \
                    patch.object(matrix.shutil, "disk_usage", return_value=SimpleNamespace(free=100 * 1024 ** 3)), \
                    patch.object(matrix, "execute_row", side_effect=execute), patch.object(matrix.subprocess, "check_output", return_value="revision"), patch("builtins.print"):
                code = matrix.main(["--parallel", "4", "--short", "--run", "run"])
            report = json.loads((root / ".meshway/validation/matrix/run/matrix.json").read_text())
            self.assertEqual(code, 1)
            self.assertEqual(len(report["rows"]), 22)
            self.assertEqual(len(builds), 3)
            self.assertTrue(all("--build-only" in command for command in builds))
            self.assertEqual(report["scope"], "diagnostic-short")

    def test_successful_fixture_with_leftover_container_fails_row(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args = matrix.parse_args(["--rows", "tun"])
            def invoke(command, logfile, env):
                report = Path(command[command.index("--report-dir") + 1])
                report.mkdir()
                (report / "report.json").write_text('{"passed": true}')
                logfile.write_text("passed")
                return 0
            with patch.object(matrix, "load", return_value={}), patch.object(matrix, "invoke", side_effect=invoke), \
                    patch.object(matrix, "remaining_resources", return_value={"containers": ["owned"], "networks": []}):
                result = matrix.execute_row("tun", directory, {"tun": Path("/bin")}, {}, args)
            self.assertFalse(result["passed"])
            self.assertEqual(result["remainingResources"]["containers"], ["owned"])

    def test_unsupported_pending_row_cannot_become_an_acceptance_pass(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(matrix.PENDING_ROWS, {"exit/off": "explicit-only: fixture"}):
            directory = Path(temporary)
            args = matrix.parse_args(["--rows", "exit/off"])
            def invoke(command, logfile, env):
                report = Path(command[command.index("--report-dir") + 1])
                report.mkdir()
                (report / "report.json").write_text('{"passed": true, "status": "unsupported"}')
                logfile.write_text("unsupported")
                return 0
            with patch.object(matrix, "load", return_value={}), patch.object(matrix, "invoke", side_effect=invoke), \
                    patch.object(matrix, "remaining_resources", return_value={"containers": [], "networks": []}):
                result = matrix.execute_row("exit/off", directory, {"runtime": Path("/bin")},
                                            {"runtime": Path("/image")}, args)
            self.assertFalse(result["passed"])
            self.assertEqual(result["status"], "unsupported")
            self.assertIn("explicit-only", result["expectation"])

    def test_exit_dns_and_fallback_are_rejected_before_docker(self):
        for options in (("--dns-backend", "resolved"), ("--traversal-fallback",)):
            result = subprocess.run([sys.executable, str(matrix.ROOT / "scripts/verify_runtime.py"),
                                     "--transport", "native-udp", "--traversal", "exit", *options],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn("requires", result.stderr)

    def test_wrong_source_artifact_rejected_before_copy(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "fixture").write_bytes(b"not an ELF")
            (directory / "source-manifest.json").write_text('{}')
            with self.assertRaisesRegex(RuntimeError, "source/target/hash"):
                import_artifact(directory, directory / "fixture", "source", "target", "linux/amd64", 62)


class RemoteTests(unittest.TestCase):
    def test_concurrent_workers_own_worktrees_share_target_and_always_remove_them(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binaries = root / ".local/bin"
            binaries.mkdir(parents=True)
            programs = {
                "git": """import pathlib,shutil,sys,json
with (pathlib.Path.home() / 'git-calls.jsonl').open('a') as log: log.write(json.dumps(sys.argv[1:]) + '\\n')
if sys.argv[1:3] == ['worktree', 'add']: pathlib.Path(sys.argv[-2]).mkdir(parents=True)
if sys.argv[1:3] == ['worktree', 'remove']: shutil.rmtree(sys.argv[-1])
print('')
""",
                "uptime": "print('fixture uptime')\n",
                "docker": "import pathlib\nprint('owned-leftover' if (pathlib.Path.home() / 'leftover').exists() else '')\n",
                # Each run waits for both to be inside the matrix: proves no run-wide lock.
                "mise": """import json,pathlib,sys,os,time
home = pathlib.Path.home()
if sys.argv[2].startswith('test:'):
    # mise run test:<kind>:linux --build-only: record where it ran, leave a frozen manifest.
    kind = sys.argv[2].split(':')[1]
    destination = pathlib.Path(sys.argv[sys.argv.index('--report-dir') + 1])
    (destination / ('bin' if kind != 'tun' else '')).mkdir(parents=True, exist_ok=True)
    (destination / ('bin' if kind != 'tun' else '') / 'source-manifest.json').write_text(json.dumps(dict(cwd=os.getcwd())))
    with (home / 'builds.jsonl').open('a') as log: log.write(json.dumps(dict(kind=kind, cwd=os.getcwd(), target=os.environ['MESHWAY_VALIDATION_TARGET'])) + '\\n')
    sys.exit(0)
run = sys.argv[sys.argv.index('--run') + 1]
(home / ('started-' + run)).touch()
deadline = time.monotonic() + 8
while len(list(home.glob('started-*'))) < 2 and time.monotonic() < deadline:
    time.sleep(0.05)
if run == 'broken':
    sys.exit(3)
directory = pathlib.Path(os.environ['MESHWAY_VALIDATION_ROOT']) / 'matrix' / run
directory.mkdir(parents=True)
data = dict(status='complete', passed=len(list(home.glob('started-*'))) == 2, sourceRoot=str(pathlib.Path.cwd()),
            target=os.environ['MESHWAY_VALIDATION_TARGET'], tmp=os.environ['TMPDIR'],
            prebuilt={kind: sys.argv[sys.argv.index('--prebuilt-' + kind) + 1] for kind in ('runtime', 'packet', 'tun')},
            rows=[dict(row='udp/off', report='rows/udp-off/actor/report.json', reportId='actor')])
(directory / 'matrix.json').write_text(json.dumps(data))
(directory / 'summary.md').write_text('matrix summary')
""",
            }
            for name, source in programs.items():
                path = binaries / name
                path.write_text('#!' + sys.executable + '\n' + source)
                path.chmod(0o700)
            environment = dict(os.environ, HOME=str(root), PATH=str(binaries) + os.pathsep + os.environ["PATH"])

            def start(name):
                job = root / ".meshway/validation/jobs" / name
                job.mkdir(parents=True)
                payload = dict(path=str(root), run=name, revision="pushed", arguments=["--short"])
                return subprocess.Popen([sys.executable, "-c", remote.WORKER, json.dumps(payload)], env=environment,
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

            def finish(names):
                for process in [start(name) for name in names]:
                    self.assertEqual(process.wait(timeout=20), 0)
                for path in root.glob("started-*"):
                    path.unlink()
                return {name: json.loads((root / ".meshway/validation/jobs" / name / "result.json").read_text())
                        for name in names}

            results = finish(["clean", "broken"])
            (root / "leftover").touch()
            results.update(finish(["leftover", "other"]))
            validation = root / ".meshway/validation"
            for name, result in results.items():
                self.assertEqual(result["status"], "complete")
                self.assertEqual(result["exitCode"] == 0, name == "clean")
                workspace = root / ".worktree" / ("validation-" + name)
                self.assertEqual(result["workspace"], str(workspace))
                self.assertTrue(result["workspaceRemoved"])
                self.assertFalse(workspace.exists())
                self.assertTrue((validation / "matrix" / name / "remote.json").exists())
                if name == "broken":
                    self.assertIn("matrix.json", result["error"])
                    continue
                data = json.loads((validation / "matrix" / name / "matrix.json").read_text())
                self.assertTrue(data["passed"], "both runs of a pair must overlap inside the matrix")
                self.assertEqual(Path(data["sourceRoot"]), workspace.resolve())
                self.assertEqual(data["target"], str(validation / "target"))
                self.assertEqual(data["tmp"], str(validation / "tmp" / name))
                self.assertEqual(data["prebuilt"]["runtime"], str(validation / "builds/pushed/runtime/bin"))
                self.assertEqual(data["prebuilt"]["tun"], str(validation / "builds/pushed/tun"))
                self.assertEqual(result["builds"]["packet"], str(validation / "builds/pushed/packet/bin"))
                if name in ("leftover", "other"):
                    self.assertIn("owned-leftover", result["remainingResources"]["containers"])
            calls = [json.loads(line) for line in (root / "git-calls.jsonl").read_text().splitlines()]
            self.assertFalse(any(call[0] == "checkout" for call in calls), "never switch the shared clone")
            build_root = str(root / ".worktree/validation-build")
            self.assertTrue(all(call[1] == build_root for call in calls if "checkout" in call), "only the build worktree is switched")
            builds = [json.loads(line) for line in (root / "builds.jsonl").read_text().splitlines()]
            # One revision: the three fixtures are built once, in the fixed-path worktree, against the shared target.
            self.assertEqual(sorted(build["kind"] for build in builds), ["packet", "runtime", "tun"])
            self.assertTrue(all(Path(build["cwd"]).resolve() == Path(build_root).resolve()
                                and build["target"] == str(validation / "target") for build in builds))
            self.assertTrue((root / ".worktree/validation-build").exists(), "the build worktree is the cache and stays")
            self.assertEqual(sum(call[:2] == ["worktree", "remove"] for call in calls), 4)
            self.assertFalse((validation / "remote.lock").exists())

    def test_forwarding_and_attach_contract(self):
        args, forwarded = remote.parse_args(["--ref", "branch", "--parallel", "4", "--short", "--rows", "nat/off"])
        self.assertEqual(args.ref, "branch")
        self.assertEqual(forwarded, ["--parallel", "4", "--short", "--rows", "nat/off"])
        for argv in (["--attach", "../other"], ["--attach", "run", "--short"], ["--run=x"], ["--host=-other"]):
            with patch("sys.stderr"), self.assertRaises(SystemExit):
                remote.parse_args(argv)

    def test_unpushed_revision_rejected_and_pushed_tip_accepted(self):
        with patch.object(remote.subprocess, "check_output", side_effect=["aaa", "aaa\trefs/heads/tooling\n"]):
            self.assertEqual(remote.pushed_revision("HEAD"), "aaa")
        with patch.object(remote.subprocess, "check_output", side_effect=["aaa", "bbb\trefs/heads/main\n"]), \
                patch.object(remote.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)):
            with self.assertRaisesRegex(RuntimeError, "push it to planck"):
                remote.pushed_revision("HEAD")

    def test_remote_programs_compile_and_launch_is_detached(self):
        compile(remote.WORKER, "worker", "exec")
        compile(remote.LAUNCH, "launch", "exec")
        self.assertIn("start_new_session=True", remote.LAUNCH)
        self.assertNotIn("remote.lock", remote.WORKER)
        self.assertNotIn("LOCK_NB", remote.WORKER)


if __name__ == "__main__":
    unittest.main()
