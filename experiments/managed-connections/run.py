#!/usr/bin/env python3
"""Acquire the shared VM lock before the Node runner can touch Docker."""

from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.dont_write_bytecode = True
sys.path.insert(0, str(HERE.parent / "transport"))
from performance_lock import locked_launch


def main():
    return locked_launch(["node", str(HERE / "run.ts"), *sys.argv[1:]],
                         fd_env="MESHWAY_MANAGED_LOCK_FD")


if __name__ == "__main__":
    sys.exit(main())
