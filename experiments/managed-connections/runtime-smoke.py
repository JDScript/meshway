#!/usr/bin/env python3
"""Validate the packaged runtime with real non-root Controller/Agent/TUN traffic."""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import uuid

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "apps/agent/tests/underlay_fixture"))
from performance_lock import LOCK_FD_ENV, performance_lock, validate_descriptor
from main_loop import normal_acceptance
from docker_lifecycle import Creation

TEMP = Path("/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode")


def run(image):
    uid, gid = os.getuid(), os.getgid()
    if uid == 0:
        raise ValueError("run as the non-root state owner")
    owner = "mw-runtime-" + uuid.uuid4().hex[:12]
    logs = TEMP / "meshway-runtime-checks" / owner
    logs.mkdir(parents=True, mode=0o700)
    result = {"status": "failed", "owner": owner, "commands": [], "cleanup_errors": []}
    containers, networks, protected, creations = [], [], {}, []
    resolved_id = None
    with performance_lock() as descriptor:
        def docker(*args, check=True, timeout=120, input=None):
            validate_descriptor(descriptor)
            args = list(map(str, args))
            creation = None
            if args[0] == "create":
                creation = Creation("container", args[args.index("--name") + 1],
                    "meshway.runtime.owner", owner, image=resolved_id,
                    network=args[args.index("--network") + 1])
            elif args[:2] == ["network", "create"]:
                creation = Creation("network", args[-1], "meshway.underlay.acceptance", owner)
            if creation:
                creations.append(creation)
            # Normal acceptance uses root exec in its base fixture. Here every
            # node command instead exercises the shipped non-root bootstrap.
            if args[0] == "exec":
                position = next((i for i, arg in enumerate(args) if arg in protected), None)
                if position is not None:
                    capabilities = "-all,+net_admin" if protected[args[position]] else "-all"
                    args[position + 1:position + 1] = ["/usr/local/bin/setpriv", "--reuid", str(uid),
                        "--regid", str(gid), "--clear-groups", "--bounding-set=" + capabilities,
                        "--inh-caps=" + capabilities, "--ambient-caps=" + capabilities, "--no-new-privs"]
            safe = args.copy()
            for i, arg in enumerate(safe[:-1]):
                if arg == "--token":
                    safe[i + 1] = "<private>"
            result["commands"].append(safe)
            def invoke():
                return subprocess.run(["docker", *args], cwd=ROOT, text=True, input=input,
                    capture_output=True, timeout=timeout, pass_fds=(descriptor,),
                    env={**os.environ, "DOCKER_CONTEXT": "default", LOCK_FD_ENV: str(descriptor)})
            completed = creation.perform(invoke) if creation else invoke()
            if check and completed.returncode:
                raise RuntimeError(completed.stderr[-4000:] + completed.stdout[-4000:])
            return completed

        def container(role, network, alias, extra=(), agent=True):
            cid = docker("create", "--name", owner + "-" + role, "--label", "meshway.runtime.owner=" + owner,
                "--network", network, "--network-alias", alias, "--cap-drop", "ALL",
                "--cap-add", "SETUID", "--cap-add", "SETGID", "--cap-add", "SETPCAP",
                *(["--cap-add", "NET_ADMIN", "--device", "/dev/net/tun",
                   "--sysctl", "net.ipv6.conf.default.disable_ipv6=1"] if agent else []),
                "--security-opt", "no-new-privileges", "--read-only", "--cpus", "1", "--memory", "384m",
                "--pids-limit", "128", "--tmpfs", f"/run:rw,nosuid,nodev,uid={uid},gid={gid},mode=0700,size=128m",
                "--tmpfs", "/tmp:rw,nosuid,nodev,size=64m", "--env", "TMPDIR=/run", *extra,
                "--entrypoint", "sleep", resolved_id, "infinity").stdout.strip()
            containers.append(cid)
            protected[cid] = agent
            inspected = json.loads(docker("inspect", cid).stdout)[0]
            assert inspected["Image"] == resolved_id
            assert not inspected["HostConfig"]["Privileged"] and inspected["HostConfig"]["NetworkMode"] != "host"
            docker("start", cid)
            code = ("import json,os; s=dict(line.split(':',1) for line in open('/proc/self/status') if ':' in line); "
                "print(json.dumps(dict(uid=os.getuid(),gid=os.getgid(),caps=s['CapEff'].strip(),nnp=s['NoNewPrivs'].strip())))")
            identity = json.loads(docker("exec", cid, "python3", "-c", code).stdout)
            assert identity == {"uid": uid, "gid": gid, "caps": "0000000000001000" if agent else "0000000000000000", "nnp": "1"}
            result.setdefault("identities", {})[role] = identity
            return cid

        try:
            resolved = json.loads(docker("image", "inspect", image).stdout)[0]
            resolved_id = resolved["Id"]
            if not re.fullmatch(r"sha256:[0-9a-f]{64}", resolved_id) or resolved["Config"].get("Volumes"):
                raise ValueError("expected an immutable runtime image without declared volumes")
            result["image"] = resolved_id
            normal_acceptance(docker, container, owner, logs, result, networks)
            result["status"] = "passed"
        except BaseException as error:
            result["error"] = str(error) or type(error).__name__
        finally:
            handlers = {sig: signal.signal(sig, signal.SIG_IGN) for sig in (signal.SIGINT, signal.SIGTERM)}
            for cid in reversed(containers):
                try:
                    value = json.loads(docker("inspect", cid).stdout)[0]
                    assert value["Config"]["Labels"].get("meshway.runtime.owner") == owner
                    for label in ("agent", "controller"):
                        log = docker("exec", cid, "cat", "/run/" + label + ".log", check=False)
                        if log.returncode == 0:
                            (logs / (value["Name"].strip("/") + "-" + label + ".log")).write_text(log.stdout)
                except Exception as error:
                    result["cleanup_errors"].append(str(error))
            for creation in reversed(creations):
                try:
                    creation.cleanup(lambda *args: docker(*args).stdout.strip())
                except Exception as error:
                    result["cleanup_errors"].append(str(error))
            result["creation_intents"] = [{"kind": item.kind, "name": item.name, "settled": item.settled}
                for item in creations]
            (logs / "result.json").write_text(json.dumps(result, indent=2))
            for sig, handler in handlers.items():
                signal.signal(sig, handler)
    print(json.dumps({"status": result["status"], "result": str(logs / "result.json"), "cleanup_errors": result["cleanup_errors"]}))
    return int(result["status"] != "passed" or bool(result["cleanup_errors"]))


def interrupted(_signal, _frame):
    raise KeyboardInterrupt("runtime acceptance interrupted")


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image")
    sys.exit(run(parser.parse_args().image))
