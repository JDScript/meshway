"""Shared process lock for development and isolated Docker jobs across worktrees."""

from contextlib import contextmanager
import fcntl
import os
from pathlib import Path
import signal
import stat
import subprocess

DEFAULT_LOCK = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock"
LOCK_FD_ENV = "MESHWAY_PERF_LOCK_FD"


def lock_path():
    path = Path(os.environ.get("MESHWAY_PERF_LOCK", DEFAULT_LOCK))
    if not path.is_absolute():
        raise ValueError("MESHWAY_PERF_LOCK must be a shared absolute private path")
    return path


def performance_environment():
    return {**os.environ, "MESHWAY_PERF_LOCK": str(lock_path())}


def validate_descriptor(descriptor):
    disk, held = lock_path().lstat(), os.fstat(descriptor)
    if not stat.S_ISREG(disk.st_mode) or (disk.st_dev, disk.st_ino) != (held.st_dev, held.st_ino):
        raise ValueError("caller must retain the shared performance lock")
    if not stat.S_ISREG(held.st_mode) or held.st_uid != os.getuid() or held.st_mode & 0o077:
        raise ValueError("Performance lock must be private and owned by the current user")
    # Reuse the same open-file description; never recursively open our own lock.
    fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)


@contextmanager
def performance_lock(inherited_env=None):
    inherited = os.environ.get(LOCK_FD_ENV)
    if inherited is None and inherited_env is not None:
        inherited = os.environ.get(inherited_env)
    if inherited is not None:
        descriptor = int(inherited)
        validate_descriptor(descriptor)
        yield descriptor
        return  # The caller owns this descriptor and its lock lifetime.
    path = lock_path()
    flags = os.O_CREAT | os.O_RDWR | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags, 0o600)
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
            raise RuntimeError("Performance lock must be private and owned by the current user")
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("Waiting for the shared Meshway performance runner…", flush=True)
            fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield descriptor
    finally:
        os.close(descriptor)


def launch(command, *, descriptor, fd_env=None):
    """Forward termination, wait for child cleanup, and inherit the held lock.

    The caller keeps its performance_lock context open throughout this call.
    fd_env additionally exports the fixture's existing Node FD contract.
    """
    validate_descriptor(descriptor)
    env = {**performance_environment(), LOCK_FD_ENV: str(descriptor)}
    if fd_env is not None:
        env[fd_env] = str(descriptor)
    child = None
    pending = []

    def forward(signum, _frame):
        if child is None:
            pending.append(signum)
        elif child.poll() is None:
            child.send_signal(signum)

    previous = {signum: signal.getsignal(signum) for signum in (signal.SIGINT, signal.SIGTERM)}
    try:
        for signum in previous:
            signal.signal(signum, forward)
        child = subprocess.Popen(command, env=env, pass_fds=(descriptor,))
        for signum in pending:
            forward(signum, None)
        return child.wait()
    finally:
        if child is not None and child.poll() is None:
            child.terminate()
            child.wait()
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def locked_launch(command, *, fd_env=None):
    """Acquire or reuse a lock, then retain it through child-owned cleanup."""
    with performance_lock(inherited_env=fd_env) as descriptor:
        return launch(command, descriptor=descriptor, fd_env=fd_env)


if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        sys.exit("Usage: performance_lock.py COMMAND [ARG ...]")
    sys.exit(locked_launch(sys.argv[1:]))
