import fcntl
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from performance_lock import DEFAULT_LOCK, launch, lock_path, performance_lock

HELPER = Path(__file__).with_name("performance_lock.py")


def line(child):
    if not select.select([child.stdout], [], [], 5)[0]:
        raise AssertionError("child handshake timed out")
    return child.stdout.readline().strip()


class PerformanceLockTests(unittest.TestCase):
    def test_default_and_explicit_path(self):
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(str(lock_path()), DEFAULT_LOCK)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "custom.lock"
            with patch.dict(os.environ, MESHWAY_PERF_LOCK=str(path)):
                with performance_lock() as descriptor:
                    self.assertEqual(os.fstat(descriptor).st_ino, path.stat().st_ino)
            with patch.dict(os.environ, MESHWAY_PERF_LOCK="relative.lock"):
                with self.assertRaises(ValueError):
                    lock_path()

    def test_two_processes_serialize_on_explicit_path(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "custom.lock"
            env = {**os.environ, "MESHWAY_PERF_LOCK": str(path)}
            command = [sys.executable, "-B", str(HELPER), sys.executable, "-u", "-c",
                       "import sys; print('acquired'); sys.stdin.read()"]
            first = subprocess.Popen(command, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
            second = None
            try:
                self.assertEqual(line(first), b"acquired")
                second = subprocess.Popen(command, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
                self.assertIn(b"Waiting", line(second))
                self.assertEqual(select.select([second.stdout], [], [], 0.1)[0], [])
                first.communicate(timeout=5)
                self.assertEqual(line(second), b"acquired")
                second.communicate(timeout=5)
                self.assertEqual(first.returncode, 0)
                self.assertEqual(second.returncode, 0)
            finally:
                for child in (first, second):
                    if child is not None and child.poll() is None:
                        child.kill()
                        child.communicate()

    def test_inherited_descriptor_is_reused_not_closed_or_reacquired(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "custom.lock"
            with patch.dict(os.environ, MESHWAY_PERF_LOCK=str(path)):
                with performance_lock() as descriptor:
                    with patch.dict(os.environ, MESHWAY_PERF_LOCK_FD=str(descriptor)), patch("performance_lock.os.open", side_effect=AssertionError("recursive lock open")):
                        with performance_lock() as inherited:
                            self.assertEqual(inherited, descriptor)
                    os.fstat(descriptor)
                    # Exercise the fixture-specific legacy FD contract too.
                    with patch.dict(os.environ, FIXTURE_LOCK_FD=str(descriptor)):
                        with performance_lock(inherited_env="FIXTURE_LOCK_FD") as inherited:
                            self.assertEqual(inherited, descriptor)
                    child_code = "from performance_lock import performance_lock; import os;\nwith performance_lock() as fd: assert fd == int(os.environ['MESHWAY_PERF_LOCK_FD'])"
                    with patch.dict(os.environ, PYTHONPATH=str(HELPER.parent)):
                        self.assertEqual(launch([sys.executable, "-B", "-c", child_code], descriptor=descriptor), 0)

    def test_inherited_descriptor_must_match_resolved_path(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict(os.environ, MESHWAY_PERF_LOCK=str(Path(directory) / "one.lock")):
                with performance_lock() as descriptor:
                    with patch.dict(os.environ, MESHWAY_PERF_LOCK=str(Path(directory) / "two.lock"), MESHWAY_PERF_LOCK_FD=str(descriptor)):
                        Path(os.environ["MESHWAY_PERF_LOCK"]).touch(mode=0o600)
                        with self.assertRaises(ValueError):
                            with performance_lock():
                                self.fail("foreign FD accepted")

    def test_sigint_and_sigterm_retain_lock_through_child_cleanup(self):
        child_code = """import signal, sys, time
def cleanup(signum, frame):
    print('cleanup', flush=True)
    sys.stdin.read()
    sys.exit(0)
signal.signal(signal.SIGINT, cleanup)
signal.signal(signal.SIGTERM, cleanup)
print('ready', flush=True)
while True: time.sleep(1)
"""
        for signum in (signal.SIGINT, signal.SIGTERM):
            with self.subTest(signal=signum), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "custom.lock"
                child = subprocess.Popen([sys.executable, "-B", str(HELPER), sys.executable, "-u", "-c", child_code],
                                         env={**os.environ, "MESHWAY_PERF_LOCK": str(path)},
                                         stdin=subprocess.PIPE, stdout=subprocess.PIPE)
                try:
                    self.assertEqual(line(child), b"ready")
                    child.send_signal(signum)
                    self.assertEqual(line(child), b"cleanup")
                    self.assertIsNone(child.poll())
                    with path.open("rb") as contender:
                        with self.assertRaises(BlockingIOError):
                            fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        child.communicate(timeout=5)
                        self.assertEqual(child.returncode, 0)
                        fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                finally:
                    if child.poll() is None:
                        child.kill()
                        child.communicate()

    def test_inherited_lock_outlives_parent_wrapper_until_child_exits(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "performance.lock"
            with patch.dict(os.environ, MESHWAY_PERF_LOCK=str(path)):
                with performance_lock() as descriptor:
                    child = subprocess.Popen(
                        [sys.executable, "-I", "-c", "import sys; sys.stdin.buffer.read()"],
                        stdin=subprocess.PIPE, pass_fds=(descriptor,),
                    )
                try:
                    with path.open("rb") as contender:
                        with self.assertRaises(BlockingIOError):
                            fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        child.communicate(timeout=5)
                        fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                finally:
                    if child.poll() is None:
                        child.kill()
                        child.communicate()


if __name__ == "__main__":
    unittest.main()
