#!/usr/bin/env python3
"""Shared launcher retains the lock through child cleanup and forwards signals."""
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1] / 'scripts'))
from performance_lock import locked_launch


if __name__ == '__main__':
    sys.exit(locked_launch([sys.executable, str(HERE / 'run.py'), *sys.argv[1:]],
                          fd_env='MESHWAY_EXIT_PERF_LOCK_FD'))
