#!/usr/bin/env python3
"""Destructive scratch-container proof for the writable Ubuntu developer image."""

import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid

LOCK = Path(os.environ.get("MESHWAY_PERF_LOCK", "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock"))


def run(args, *, timeout=120, output=True):
    return subprocess.run(args, check=True, text=True, timeout=timeout,
                          stdout=subprocess.PIPE if output else None).stdout if output else ""


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: dev-shell-smoke.py IMAGE")
    uid, gid = os.getuid(), os.getgid()
    if uid == 0:
        raise RuntimeError("run as the private-state owner, not root")
    name = f"meshway-dev-smoke-{uuid.uuid4().hex[:12]}"
    with LOCK.open("a+") as lock, tempfile.TemporaryDirectory(prefix="meshway-dev-smoke-") as state:
        os.chmod(state, 0o700)
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            run(["docker", "create", "--name", name, "--pull", "never", "--user", "0:0",
                 "--cap-add", "NET_ADMIN", "--security-opt", "no-new-privileges",
                 "--network", "bridge", "--mount", f"type=bind,src={state},dst=/state",
                 "--env", f"MESHWAY_AGENT_UID={uid}", "--env", f"MESHWAY_AGENT_GID={gid}",
                 "--entrypoint", "sleep", sys.argv[1], "infinity"])
            run(["docker", "start", name])
            inspect = json.loads(run(["docker", "inspect", name]))[0]
            assert inspect["HostConfig"]["ReadonlyRootfs"] is False
            assert inspect["HostConfig"]["Privileged"] is False
            assert inspect["HostConfig"]["CapAdd"] == ["NET_ADMIN"]
            assert inspect["HostConfig"]["SecurityOpt"] in (["no-new-privileges"], ["no-new-privileges=true"])
            assert run(["docker", "exec", name, "sh", "-c", ". /etc/os-release; test \"$ID:$VERSION_ID\" = ubuntu:24.04; command -v bash curl ping ip python3 setpriv traceroute"]) is not None
            run(["docker", "exec", name, "bash", "-lc", "apt-get update >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends netcat-openbsd >/dev/null && nc -h >/dev/null 2>&1 || test $? = 1"], timeout=300)
            run(["docker", "exec", name, "traceroute", "-m", "1", "127.0.0.1"])
            run(["docker", "exec", name, "traceroute", "-I", "-m", "1", "127.0.0.1"])
            proof = run(["docker", "exec", name, "meshway", "--help"])
            assert "Meshway" in proof
            owner = run(["docker", "exec", name, "sh", "-c", "meshway --help >/dev/null; stat -c '%u:%g' /state"]).strip()
            assert owner == f"{uid}:{gid}"
            assert not list(Path(state).iterdir()), "CLI help must not create root-owned state"
            print(json.dumps({"image": inspect["Image"], "ubuntu": "24.04", "rootfs": "writable",
                              "root_apt_install": "netcat-openbsd", "agent_cli_uid": uid,
                              "cap_add": inspect["HostConfig"]["CapAdd"]}))
        finally:
            subprocess.run(["docker", "rm", "--force", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    main()
