#!/usr/bin/env python3
"""Hold the shared Docker performance lock for the complete acceptance run."""

from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.dont_write_bytecode = True
sys.path.insert(0, str(HERE.parents[1] / "scripts"))
from performance_lock import locked_launch


def main():
    return locked_launch(["node", str(HERE / "run.ts"), *sys.argv[1:]],
                         fd_env="MESHWAY_DATAPLANE_LOCK_FD")


if __name__ == "__main__":
    sys.exit(main())
