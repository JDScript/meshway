#!/usr/bin/env python3
"""Hold the shared VM lock through owned cleanup; accept an already-held caller FD."""
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1] / "scripts"))
from performance_lock import locked_launch


def main():
    if "--help" in sys.argv:
        return subprocess.call(["node", str(HERE / "run.ts"), "--help"])
    return locked_launch(["node", str(HERE / "run.ts"), *sys.argv[1:]],
                         fd_env="MESHWAY_DISCOVERY_LOCK_FD")


if __name__ == "__main__":
    sys.exit(main())
