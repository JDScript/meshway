#!/usr/bin/env python3
"""Acquire the shared VM lock before the Node runner can touch Docker."""

import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.dont_write_bytecode = True
sys.path.insert(0, str(HERE.parent / "transport"))
from performance_lock import performance_lock

DEFAULT_LOCK = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock"


def main():
    os.environ.setdefault("MESHWAY_PERF_LOCK", DEFAULT_LOCK)
    # Pass the very same flock description into Node, matching the shared helper.
    with performance_lock() as descriptor:
        env = {**os.environ, "MESHWAY_MANAGED_LOCK_FD": str(descriptor)}
        return subprocess.run(
            ["node", str(HERE / "run.ts"), *sys.argv[1:]],
            env=env, pass_fds=(descriptor,), check=False,
        ).returncode


if __name__ == "__main__":
    sys.exit(main())
