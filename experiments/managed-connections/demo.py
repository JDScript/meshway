#!/usr/bin/env python3
"""Locked host launcher and stdin-only, container-local enrollment helper."""

import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys

DEFAULT_LOCK = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock"


def checked(arguments):
    result = subprocess.run(arguments, capture_output=True, text=True, timeout=30, check=False)
    if result.returncode:
        raise RuntimeError("Agent command failed; private output suppressed")
    return result.stdout


def container_enroll(data):
    """Never mint a replacement credential for an existing state file.

    The original grant arrives on stdin, not host Docker argv. The real Agent
    persists the proposed credential before enrollment. Heartbeat can recover a
    committed enrollment whose response was lost, even after the grant expires.
    """
    os.umask(0o077)
    if os.getuid() == 0:
        raise RuntimeError("non-root Agent required")
    state = Path("/state/agent.json")
    name, controller, token = data["name"], data["controller"], data["token"]
    if name not in ("demo-nyc", "demo-lax", "demo-sha") or not isinstance(controller, str):
        raise RuntimeError("invalid binding")
    if token is not None and (not isinstance(token, str) or re.fullmatch(r"[0-9a-f]{64}", token) is None):
        raise RuntimeError("invalid original grant")

    def status():
        value = json.loads(checked(["meshway-agent", "status", "--state", str(state)]))
        if value["name"] != name or value["controller"] != controller:
            raise RuntimeError("different immutable binding")
        node_id = value["node_id"]
        if node_id is not None and re.fullmatch(r"[0-9a-f]{64}", node_id) is None:
            raise RuntimeError("invalid identity")
        if data["expected_id"] is not None and node_id != data["expected_id"]:
            raise RuntimeError("identity differs from demo metadata")
        return node_id

    if data["operation"] == "publish":
        node_id = status()
        if node_id is None:
            raise RuntimeError("cannot publish a pending identity")
        connection = json.loads(checked(["meshway-agent", "connections", "status", "--state", str(state)]))
        expected_key = data["expected_key"]
        if expected_key is not None and connection["public_key"] != expected_key:
            raise RuntimeError("bound connection key missing or changed; refusing regeneration")
        checked(["meshway-agent", "connections", "publish", "--state", str(state)])
        return {"node_id": node_id}
    if data["operation"] != "recover":
        raise RuntimeError("invalid helper operation")
    if state.exists() or state.is_symlink():
        before = status()  # Real CLI validates private state, not just JSON shape.
        try:
            checked(["meshway-agent", "heartbeat", "--state", str(state)])
        except Exception:
            # An enrolled/revoked/unrecognized identity never falls back to enroll.
            if before is not None or token is None:
                raise
            checked(["meshway-agent", "enroll", "--controller", controller,
                     "--token", token, "--name", name, "--state", str(state)])
    else:
        if token is None or data["expected_id"] is not None:
            raise RuntimeError("missing original enrollment state")
        checked(["meshway-agent", "enroll", "--controller", controller,
                 "--token", token, "--name", name, "--state", str(state)])
    node_id = status()
    if node_id is None:
        raise RuntimeError("enrollment is still pending")
    return {"node_id": node_id}


def main():
    if sys.argv[1:2] == ["--container-enroll"]:
        try:
            print(json.dumps(container_enroll(json.load(sys.stdin))))
            return 0
        except Exception:
            print("demo enrollment failed; private diagnostics suppressed", file=sys.stderr)
            return 1

    here = Path(__file__).resolve().parent
    if "--help" in sys.argv[1:]:
        return subprocess.run(["node", str(here / "demo.ts"), "--help"], check=False).returncode
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(here.parent / "transport"))
    from performance_lock import performance_lock

    os.environ.setdefault("MESHWAY_PERF_LOCK", DEFAULT_LOCK)
    with performance_lock() as descriptor:
        child = subprocess.Popen(
            ["node", str(here / "demo.ts"), *sys.argv[1:]],
            env={**os.environ, "MESHWAY_DEMO_LOCK_FD": str(descriptor)},
            pass_fds=(descriptor,),
        )
        while True:
            try:
                return child.wait()
            except KeyboardInterrupt:
                # Node finishes its bounded command and stops owned containers;
                # keep the inherited lock alive throughout that cleanup.
                child.send_signal(signal.SIGINT)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        print("demo launcher failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
