"""Matrix orchestration regressions; no real builds, Docker, SSH or measurements."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import validate_matrix as matrix
import validate_remote as remote
from validation_support import import_artifact


class MatrixTests(unittest.TestCase):
    def test_rows_are_nine_and_filter_keeps_order_without_duplicates(self):
        self.assertEqual(len(matrix.ROWS), 9)
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
            self.assertEqual("--traversal-image-manifest" in command, name.startswith("nat"))
            self.assertEqual("--short" in command, name != "tun")
            if name == "packet":
                self.assertIn("--runtime-artifacts", command)

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
                    patch.object(matrix, "execute_row", side_effect=execute), patch.object(matrix.subprocess, "check_output", return_value="revision"), patch("builtins.print"):
                code = matrix.main(["--parallel", "4", "--short", "--run", "run"])
            report = json.loads((root / ".meshway/validation/matrix/run/matrix.json").read_text())
            self.assertEqual(code, 1)
            self.assertEqual(len(report["rows"]), 9)
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

    def test_wrong_source_artifact_rejected_before_copy(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "fixture").write_bytes(b"not an ELF")
            (directory / "source-manifest.json").write_text('{}')
            with self.assertRaisesRegex(RuntimeError, "source/target/hash"):
                import_artifact(directory, directory / "fixture", "source", "target", "linux/amd64", 62)


class RemoteTests(unittest.TestCase):
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
        self.assertIn("LOCK_EX | fcntl.LOCK_NB", remote.WORKER)


if __name__ == "__main__":
    unittest.main()
