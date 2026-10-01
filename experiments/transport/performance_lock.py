"""Serialize performance jobs across worktrees sharing one Docker engine."""

from contextlib import contextmanager
import fcntl
import os
from pathlib import Path
import tempfile


@contextmanager
def performance_lock():
    path = Path(os.environ.get("MESHWAY_PERF_LOCK", str(Path(tempfile.gettempdir()) / "meshway-performance.lock")))
    flags = os.O_CREAT | os.O_RDWR | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags, 0o600)
    try:
        metadata = os.fstat(descriptor)
        if metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
            raise RuntimeError("Performance lock must be private and owned by the current user")
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("Waiting for the shared Meshway performance runner…", flush=True)
            fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield descriptor
    finally:
        os.close(descriptor)


if __name__ == "__main__":
    import subprocess
    import sys

    if len(sys.argv) < 2:
        sys.exit("Usage: performance_lock.py COMMAND [ARG ...]")
    with performance_lock() as descriptor:
        # The child retains the same flock even if this wrapper is terminated.
        sys.exit(subprocess.run(sys.argv[1:], check=False, pass_fds=(descriptor,)).returncode)
