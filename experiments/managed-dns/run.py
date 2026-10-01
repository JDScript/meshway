#!/usr/bin/env python3
"""Keep the shared performance lock until Node completes owned-resource cleanup."""
import os
from pathlib import Path
import signal
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "transport"))
from performance_lock import performance_lock

LOCK = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock"


def main():
    # All Meshway fixtures sharing this VM must use exactly this lock.
    os.environ["MESHWAY_PERF_LOCK"] = LOCK
    with performance_lock() as descriptor:
        child = subprocess.Popen(["node", str(HERE / "run.ts"), *sys.argv[1:]],
                                 env={**os.environ, "MESHWAY_DNS_LOCK_FD": str(descriptor)},
                                 pass_fds=(descriptor,))

        def forward(signum, _frame):
            if child.poll() is None:
                child.send_signal(signum)

        signal.signal(signal.SIGINT, forward)
        signal.signal(signal.SIGTERM, forward)
        return child.wait()


if __name__ == "__main__":
    sys.exit(main())
