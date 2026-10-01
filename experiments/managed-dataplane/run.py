#!/usr/bin/env python3
"""Hold the shared Docker performance lock for the complete acceptance run."""

import os
from pathlib import Path
import signal
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.dont_write_bytecode = True
sys.path.insert(0, str(HERE.parent / "transport"))
from performance_lock import performance_lock

DEFAULT_LOCK = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock"


def main():
    os.environ.setdefault("MESHWAY_PERF_LOCK", DEFAULT_LOCK)
    with performance_lock() as descriptor:
        child = subprocess.Popen(
            ["node", str(HERE / "run.ts"), *sys.argv[1:]],
            env={**os.environ, "MESHWAY_DATAPLANE_LOCK_FD": str(descriptor)},
            pass_fds=(descriptor,),
        )
        # tmux/main owns process lifetime. Forward termination and retain the
        # inherited flock until Node has finished its owned-resource cleanup.
        def forward(signum, _frame):
            if child.poll() is None:
                child.send_signal(signum)
        signal.signal(signal.SIGINT, forward)
        signal.signal(signal.SIGTERM, forward)
        return child.wait()


if __name__ == "__main__":
    sys.exit(main())
