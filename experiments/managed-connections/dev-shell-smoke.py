#!/usr/bin/env python3
"""Destructive scratch-container proof for the writable Ubuntu developer image."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1] / "scripts"))
from performance_lock import performance_lock

TEMP = Path("/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode")


def run(args, *, timeout=120):
    return subprocess.run(args, check=True, text=True, timeout=timeout,
                          stdout=subprocess.PIPE).stdout


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: dev-shell-smoke.py IMAGE")
    uid, gid = os.getuid(), os.getgid()
    if uid == 0:
        raise RuntimeError("run as the private-state owner, not root")
    name = f"meshway-dev-smoke-{uuid.uuid4().hex[:12]}"
    # Docker Desktop/OrbStack may not share the macOS private temporary tree.
    # A private, disposable directory in the shared workspace exercises the
    # same bind semantics as the persistent demo without mounting its state.
    scratch_parent = HERE.parents[1] / ".meshway"
    scratch_parent.mkdir(mode=0o700, exist_ok=True)
    created = False
    with performance_lock(), tempfile.TemporaryDirectory(prefix="dev-shell-smoke-", dir=scratch_parent) as state:
        os.chmod(state, 0o700)
        try:
            run(["docker", "create", "--name", name, "--pull", "never", "--user", "0:0",
                 "--label", f"io.meshway.dev-smoke.owner={name}",
                 "--cap-add", "NET_ADMIN", "--security-opt", "no-new-privileges",
                 "--network", "bridge", "--mount", f"type=bind,src={state},dst=/state",
                 "--env", f"MESHWAY_AGENT_UID={uid}", "--env", f"MESHWAY_AGENT_GID={gid}",
                  "--entrypoint", "sleep", sys.argv[1], "infinity"])
            created = True
            run(["docker", "start", name])
            inspect = json.loads(run(["docker", "inspect", name]))[0]
            assert inspect["HostConfig"]["ReadonlyRootfs"] is False
            assert inspect["HostConfig"]["Privileged"] is False
            assert [cap.removeprefix("CAP_") for cap in inspect["HostConfig"]["CapAdd"]] == ["NET_ADMIN"]
            assert inspect["HostConfig"]["SecurityOpt"] in (["no-new-privileges"], ["no-new-privileges=true"])
            assert run(["docker", "exec", name, "sh", "-c", ". /etc/os-release; test \"$ID:$VERSION_ID\" = ubuntu:24.04; command -v bash curl ping ip python3 setpriv traceroute"]) is not None
            run(["docker", "exec", name, "apt-get", "update", "--error-on=any"], timeout=300)
            run(["docker", "exec", "--env", "DEBIAN_FRONTEND=noninteractive", name,
                 "apt-get", "install", "-y", "--no-install-recommends", "netcat-openbsd"], timeout=300)
            assert run(["docker", "exec", name, "dpkg-query", "-W", "-f=${Status}", "netcat-openbsd"]).strip() == "install ok installed"
            assert run(["docker", "exec", name, "sh", "-c", "command -v nc"]).strip()
            run(["docker", "exec", name, "traceroute", "-m", "1", "127.0.0.1"])
            run(["docker", "exec", name, "traceroute", "-I", "-m", "1", "127.0.0.1"])
            proof = run(["docker", "exec", name, "meshway", "--help"])
            assert "Meshway" in proof
            # OrbStack maps bind ownership according to the calling UID; verify
            # the view used by the non-root Agent rather than the root shell.
            owner = run(["docker", "exec", "--user", f"{uid}:{gid}", name,
                         "stat", "-c", "%u:%g", "/state"]).strip()
            expected_owner = f"{os.stat(state).st_uid}:{os.stat(state).st_gid}"
            assert owner == expected_owner, f"bind owner {owner}, host owner {expected_owner}"
            assert not list(Path(state).iterdir()), "CLI help must not create root-owned state"
            # In this disposable writable layer only, substitute a UID/capability
            # probe for the binary to prove the launcher's effective identity.
            identity = json.loads(run(["docker", "exec", name, "python3", "-c", '''import json, os, pathlib, subprocess
p = pathlib.Path("/usr/local/bin/meshway-agent")
original = p.read_bytes()
mode = p.stat().st_mode & 0o777
try:
    p.write_text("#!/usr/bin/python3\\nimport json,os\\ns=dict(line.split(':',1) for line in open('/proc/self/status') if ':' in line)\\nprint(json.dumps(dict(uid=os.getuid(),gid=os.getgid(),caps=s['CapEff'].strip())))\\n")
    p.chmod(0o755)
    print(subprocess.check_output(["meshway", "--help"], text=True).strip())
finally:
    p.write_bytes(original)
    p.chmod(mode)
''']))
            assert identity == {"uid": uid, "gid": gid, "caps": "0000000000000000"}
            print(json.dumps({"image": inspect["Image"], "ubuntu": "24.04", "rootfs": "writable",
                              "root_apt_install": "netcat-openbsd", "agent_cli_uid": uid,
                              "cap_add": inspect["HostConfig"]["CapAdd"]}))
        finally:
            if created:
                value = json.loads(run(["docker", "inspect", name]))[0]
                assert value["Config"]["Labels"]["io.meshway.dev-smoke.owner"] == name
                run(["docker", "rm", "--force", name])


if __name__ == "__main__":
    main()
