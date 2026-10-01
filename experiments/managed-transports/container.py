#!/usr/bin/env python3
"""Private isolated acceptance helper, never exports secret material or logs."""
import importlib.util
import json
import os
from pathlib import Path
import sys
sys.dont_write_bytecode = True
os.umask(0o077)
spec = importlib.util.spec_from_file_location("dataplane", "/private/dataplane.py")
dataplane = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dataplane)
base = dataplane.base
ROOT = Path("/private")

def main(action, data):
    if action == "controller-start":
        return base.start("controller", ["meshway-controller", "serve", "--bind", "0.0.0.0:8420", "--data-dir", "/private/controller", "--ipv4-prefix", "100.127.240.0/24", "--ipv6-prefix", "fd62:bfaa:3401::/64"])
    if action == "enroll":
        if base.STATE.exists() or data["name"] not in ("NYC", "SHA"):
            raise ValueError("invalid fixture enrollment")
        base.checked(["meshway-agent", "enroll", "--controller", "http://controller.mesh.test:8420", "--token", data["token"], "--name", data["name"], "--state", str(base.STATE)], timeout=30)
        base.checked(["meshway-agent", "connections", "publish", "--state", str(base.STATE)], timeout=30)
        return {"node_id": base.identity()["node_id"]}
    if action == "agent-start":
        (ROOT / "agent.log").write_text("")
        return base.start("agent", ["meshway-agent", "run", "--state", str(base.STATE), "--apply-dataplane", "--tun-name", "meshway0", "--tun-mtu", "1000", "--interval-seconds", "2", "--connection-lease-seconds", "15"])
    if action == "material-status":
        journal = ROOT / "agent.json.transport-materials.json"
        return {"journal_saved": journal.exists(), "journal_private": journal.exists() and journal.stat().st_mode & 0o777 == 0o600,
                "legacy_profiles_absent": not (ROOT / "profiles.json").exists()}
    return dataplane.main(action, data)

if __name__ == "__main__":
    try:
        if sys.argv[1] == "http-server":
            dataplane.serve()
        else:
            print(json.dumps(main(sys.argv[1], json.load(sys.stdin))))
    except Exception:
        print("managed transport fixture failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
