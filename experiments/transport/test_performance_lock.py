import fcntl
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from performance_lock import performance_lock


class PerformanceLockTests(unittest.TestCase):
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
