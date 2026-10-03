#!/usr/bin/env python3
from pathlib import Path
import sys
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from performance_lock import locked_launch
sys.exit(locked_launch(["node", str(Path(__file__).with_suffix(".ts")), *sys.argv[1:]], fd_env="MESHWAY_DATAPLANE_LOCK_FD"))
