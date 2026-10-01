#!/usr/bin/env python3
"""Keep the shared performance lock until Node completes owned-resource cleanup."""
from pathlib import Path
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "transport"))
from performance_lock import locked_launch


def main():
    return locked_launch(["node", str(HERE / "run.ts"), *sys.argv[1:]],
                         fd_env="MESHWAY_DNS_LOCK_FD")


if __name__ == "__main__":
    sys.exit(main())
