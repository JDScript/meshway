#!/usr/bin/env python3
"""Real Xray interop under the shared Docker VM performance lock. No benchmark."""

import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "transport"))
from performance_lock import performance_lock  # noqa: E402


def run_test(command, environment):
    # Kill the complete cargo/test/Docker-client process group on cancellation.
    # Killing only cargo can leave its test process and temporary listeners alive.
    process = subprocess.Popen(command, cwd=ROOT, env=environment, start_new_session=True)
    try:
        code = process.wait(timeout=240)
        if code:
            raise subprocess.CalledProcessError(code, command)
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        raise


def main():
    if "MESHWAY_PERF_LOCK" not in os.environ:
        raise SystemExit("Set MESHWAY_PERF_LOCK to the shared Meshway performance lock path")
    # Host build does not occupy the shared Docker VM.
    command = ["cargo", "test", "-p", "meshway-transport-trojan", "--test", "xray"]
    subprocess.run(command + ["--no-run", "--locked"], cwd=ROOT, check=True)
    name = "meshway-trojan-interop-" + uuid.uuid4().hex[:16]
    with performance_lock(), tempfile.TemporaryDirectory(prefix=name, dir=ROOT / "target") as temporary:
        environment = dict(os.environ, MESHWAY_TROJAN_INTEROP_CONTAINER=name,
                           MESHWAY_TROJAN_TEMP_ROOT=temporary)
        try:
            run_test(command + ["--locked", "--", "--ignored", "--nocapture", "--test-threads=1"], environment)
        finally:
            # Also covers a killed/timed-out Rust process, where Drop cannot run.
            subprocess.run(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=30, check=False)


if __name__ == "__main__":
    main()
