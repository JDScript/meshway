#!/usr/bin/env python3
"""Serialize the complete isolated functional acceptance against shared Docker."""
from pathlib import Path
import sys
sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "transport"))
from performance_lock import locked_launch
sys.exit(locked_launch(["node", str(HERE / "run.ts"), *sys.argv[1:]], fd_env="MESHWAY_DATAPLANE_LOCK_FD"))
