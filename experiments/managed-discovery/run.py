#!/usr/bin/env python3
"""Hold the shared VM lock through owned cleanup; accept an already-held caller FD."""
import os
import fcntl
from pathlib import Path
import signal
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "transport"))
from performance_lock import performance_lock

LOCK = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock"


def launch(descriptor):
    disk, held = os.stat(LOCK), os.fstat(descriptor)
    if (disk.st_dev, disk.st_ino) != (held.st_dev, held.st_ino):
        raise ValueError("caller must retain the shared performance lock")
    if held.st_uid != os.getuid() or held.st_mode & 0o077:
        raise ValueError("performance lock must be private and owned")
    # flock on the inherited open-file description preserves a caller's lock;
    # never open a second description and recursively wait on our own lock.
    fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
    child = subprocess.Popen(["node", str(HERE / "run.ts"), *sys.argv[1:]],
                             env={**os.environ, "MESHWAY_DISCOVERY_LOCK_FD": str(descriptor)},
                             pass_fds=(descriptor,))

    def forward(signum, _frame):
        if child.poll() is None:
            child.send_signal(signum)

    signal.signal(signal.SIGINT, forward)
    signal.signal(signal.SIGTERM, forward)
    return child.wait()


def main():
    if "--help" in sys.argv:
        return subprocess.call(["node", str(HERE / "run.ts"), "--help"])
    os.environ["MESHWAY_PERF_LOCK"] = LOCK
    inherited = os.environ.get("MESHWAY_DISCOVERY_LOCK_FD")
    if inherited is not None:
        return launch(int(inherited))
    with performance_lock() as descriptor:
        return launch(descriptor)


if __name__ == "__main__":
    sys.exit(main())
