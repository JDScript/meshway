#!/usr/bin/env python3
"""Hold the shared Docker lock through build, workload and owned cleanup."""
from pathlib import Path
import sys
sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1] / "scripts"))
from performance_lock import locked_launch
if __name__ == "__main__":
    sys.exit(locked_launch(["node", str(HERE / "run.ts"), *sys.argv[1:]],
                          fd_env="MESHWAY_MANAGED_TRANSPORTS_LOCK_FD"))
