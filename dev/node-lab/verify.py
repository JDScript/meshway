#!/usr/bin/env python3
"""Explicit isolated Ubuntu OS/ownership check; never builds or enrolls a Node."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import node_lab  # noqa: E402


def run(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT, timeout=45).strip()


def wait(check, seconds=30):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except subprocess.CalledProcessError:
            pass
        time.sleep(.2)
    raise RuntimeError("Isolated OS verification timed out.")


def verify(args):
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    # Reuse exactly the deployment image verifier with a private local selection.
    lab = output / "selection"
    image_dir = lab / "images/verified"
    image_dir.mkdir(parents=True, mode=0o700)
    shutil.copyfile(args.image_manifest, image_dir / "image-manifest.json")
    record = json.loads(args.image_manifest.read_text())
    node_lab.save(lab / "selected-image.json", {"manifest": str(image_dir / "image-manifest.json")})
    image = node_lab.selected_image(lab, record["platform"])
    release = args.release.resolve()
    manifest = json.loads((release / "build.json").read_text())
    if manifest["platform"] != record["platform"]:
        raise RuntimeError("Binary release and OS architecture differ.")
    for name in ("meshwayd", "meshway"):
        node_lab.check_elf(release / name, {"linux/arm64": 183, "linux/amd64": 62}[record["platform"]])
        if node_lab.sha256(release / name) != manifest["sha256"][name]:
            raise RuntimeError("Existing release checksum differs.")
    identity = "meshway-lab-os-" + uuid.uuid4().hex[:16]
    label = "io.meshway.node-lab.verify=" + identity
    resources = []
    observer = None
    evidence = {"image": image["imageId"], "binaryRelease": manifest, "verification": "OS services and process ownership only"}

    def execute(*command):
        return run("docker", "exec", identity, *command)

    def inspect():
        return json.loads(run("docker", "container", "inspect", identity))[0]

    def status():
        if not inspect()["State"]["Running"]:
            raise RuntimeError("OS service owner exited before IPC readiness: " + run("docker", "logs", identity))
        return json.loads(execute("/opt/meshway/meshway", "status", "--json"))

    def stopped():
        state = inspect()["State"]
        return state if not state["Running"] else None

    try:
        negative = identity + "-no-admin"
        run("docker", "create", "--name", negative, "--label", label, "--network", "none",
            "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
            "--env", "MESHWAY_NODE_LAB_OWNED=1", image["imageId"])
        resources.append(("container", negative))
        run("docker", "start", negative)
        wait(lambda: not json.loads(run("docker", "container", "inspect", negative))[0]["State"]["Running"])
        negative_state = json.loads(run("docker", "container", "inspect", negative))[0]["State"]
        evidence["withoutSysAdmin"] = run("docker", "logs", negative)
        assert negative_state["ExitCode"] != 0 and "Operation not permitted" in evidence["withoutSysAdmin"]
        run("docker", "network", "create", "--label", label, identity)
        resources.append(("network", identity))
        run("docker", "volume", "create", "--label", label, identity)
        resources.append(("volume", identity))
        command = ["docker", "create", "--name", identity, "--hostname", identity, "--label", label,
                   "--network", identity, "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
                   "--memory", "384m", "--cpus", "0.5", "--pids-limit", "64",
                   "--tmpfs", "/run:rw,nosuid,nodev,noexec,size=16m,mode=755",
                   "--sysctl", "net.ipv6.conf.all.disable_ipv6=0", "--sysctl", "net.ipv6.conf.default.disable_ipv6=0",
                   "--device", "/dev/net/tun:/dev/net/tun", "--env", "MESHWAY_NODE_LAB_OWNED=1",
                   "--mount", f"type=bind,src={release},dst=/opt/meshway,readonly",
                   "--mount", f"type=volume,src={identity},dst=/var/lib/meshway"]
        # Obtain the same cap contract as Compose without running Compose up.
        env = dict(os.environ, MESHWAY_NODE_IMAGE=image["imageId"], MESHWAY_NODE_RELEASE=str(release),
                   MESHWAY_NODE_PLATFORM=record["platform"])
        config = json.loads(node_lab.compose(env, "config", "--format", "json", capture=True))["services"]["lax"]
        for cap in config["cap_add"]:
            command += ["--cap-add", cap]
        # A read-only observer stays in Docker's original namespace while PID 1
        # execs the real entrypoint. It proves the original binds never changed.
        command += ["--entrypoint", "/bin/sh", image["imageId"], "-ec",
                    'while [ ! -f /root/verify-go ]; do sleep .1; done; exec /usr/local/lib/meshway-lab/entrypoint.sh "$@"',
                    "lab-verify", "--data-dir", "/var/lib/meshway", "--server", "http://127.0.0.1:8430",
                    "--network-mode", "tun", "--bootstrap-nameserver", "1.1.1.1:53"]
        run(*command)
        resources.append(("container", identity))
        run("docker", "start", identity)
        observer = subprocess.Popen(["docker", "exec", identity, "/bin/sh", "-ec",
            'sha256sum /etc/hosts /etc/hostname /etc/resolv.conf >/run/verify-original; '
            'readlink /proc/self/ns/mnt >/run/verify-original-ns; '
            'touch /run/verify-observing; '
            'while [ ! -f /run/verify-finished ]; do sleep .1; done; '
            'sha256sum -c /run/verify-original'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        wait(lambda: execute("test", "-f", "/run/verify-observing") == "")
        execute("touch", "/root/verify-go")
        current = wait(status)
        proof = node_lab.daemon_proof("isolated", inspect(), current)
        assert proof["processId"] != 1
        evidence["daemonProof"] = proof
        evidence["namespace"] = json.loads(execute("cat", "/run/meshway-lab/namespace.json"))
        assert evidence["namespace"]["parentMount"] == execute("cat", "/run/verify-original-ns")
        assert evidence["namespace"]["mount"] != evidence["namespace"]["parentMount"]
        evidence["mounts"] = execute("cat", "/proc/1/mountinfo")
        assert not any("shared:" in line or "master:" in line for line in evidence["mounts"].splitlines())
        assert execute("readlink", "/etc/resolv.conf") == "/run/systemd/resolve/stub-resolv.conf"
        evidence["resolved"] = execute("resolvectl", "status")
        assert "resolv.conf mode: stub" in evidence["resolved"]
        evidence["versions"] = execute("/bin/sh", "-ec",
            'cat /etc/os-release; ping -V; traceroute --version; iperf --version; iperf3 --version; '
            'ip -Version; dig -v; curl --version; resolvectl --version; dbus-daemon --version')
        assert 'VERSION_ID="24.04"' in evidence["versions"]
        evidence["ping4"] = execute("ping", "-4", "-c", "1", "127.0.0.1")
        evidence["ping6"] = execute("ping", "-6", "-c", "1", "::1")
        evidence["traceroute"] = execute("traceroute", "-n", "-m", "1", "-w", "1", "127.0.0.1")
        evidence["listeners"] = execute("ss", "-lntup")
        assert ":5001 " not in evidence["listeners"] and ":5201 " not in evidence["listeners"]
        execute("/bin/sh", "-ec", 'echo writable >/root/lab-writable; test ! -w /opt/meshway/meshwayd')
        evidence["stateMode"] = execute("stat", "-c", "%u:%g:%a", "/var/lib/meshway")
        assert evidence["stateMode"] == "0:0:700"
        evidence["daemonCaps"] = execute("grep", "Cap", f"/proc/{proof['processId']}/status")
        for line in evidence["daemonCaps"].splitlines():
            if line.startswith(("CapEff:", "CapBnd:")):
                assert not int(line.split()[1], 16) & (1 << 21)  # SYS_ADMIN
        services = json.loads(execute("cat", "/run/meshway-lab/services.json"))
        # This is also the kernel access required by the production DNS owner's
        # resolve1 process/network-namespace binding check, after its UID drop.
        evidence["resolvedNetwork"] = execute("readlink", f"/proc/{services['resolved']}/ns/net")
        assert evidence["resolvedNetwork"] == evidence["namespace"]["network"]
        try:
            execute("setpriv", "--bounding-set=-sys_ptrace", "--", "readlink", "-v", f"/proc/{services['resolved']}/ns/net")
        except subprocess.CalledProcessError as error:
            evidence["withoutSysPtrace"] = error.output
            assert "Permission denied" in error.output
        else:
            raise RuntimeError("SYS_PTRACE was unnecessary for this image; narrow the capability contract.")
        execute("resolvectl", "domain", "eth0", "com")
        evidence["searchStub"] = execute("cat", "/etc/resolv.conf")
        assert "search com" in evidence["searchStub"]
        evidence["systemSearch"] = execute("getent", "ahostsv4", "example")
        assert "example.com" in evidence["systemSearch"]
        execute("resolvectl", "revert", "eth0")
        assert "search com" not in execute("cat", "/etc/resolv.conf")
        execute("/opt/meshway/meshway", "down", "--json")
        assert status()["dataPlane"] == "stopped"
        execute("touch", "/run/verify-finished")
        out, err = observer.communicate(timeout=10)
        assert observer.returncode == 0, err
        evidence["originalDockerBinds"] = out
        # Real daemon termination must make the owner join resolved and dbus.
        execute("kill", "-TERM", str(proof["processId"]))
        evidence["daemonExit"] = wait(stopped)
        assert evidence["daemonExit"]["ExitCode"] == 0
        run("docker", "start", identity)
        restarted = wait(status)
        evidence["restartProof"] = node_lab.daemon_proof("isolated", inspect(), restarted)
        assert evidence["restartProof"]["mountNamespace"] != proof["mountNamespace"]
        assert execute("cat", "/root/lab-writable") == "writable"
        assert "search com" not in execute("cat", "/etc/resolv.conf")
        assert "resolv.conf mode: stub" in execute("resolvectl", "status")
        services = json.loads(execute("cat", "/run/meshway-lab/services.json"))
        execute("kill", "-TERM", str(services["resolved"]))
        evidence["requiredServiceExit"] = wait(stopped)
        assert evidence["requiredServiceExit"]["ExitCode"] != 0
        run("docker", "start", identity)
        wait(status)
        run("docker", "stop", "--time", "-1", identity)
        evidence["managedStop"] = wait(stopped)
        assert evidence["managedStop"]["ExitCode"] == 0
        evidence["passed"] = True
    finally:
        if ("container", identity) in resources:
            evidence["logs"] = run("docker", "logs", identity)
        for kind, name in reversed(resources):
            value = json.loads(run("docker", kind, "inspect", name))[0]
            labels = value["Config"]["Labels"] if kind == "container" else value["Labels"]
            if labels.get("io.meshway.node-lab.verify") != identity:
                raise RuntimeError("Temporary resource ownership changed; retained.")
            run("docker", kind, "rm", *(["-f"] if kind == "container" else []), name)
        if observer is not None:
            observer.communicate(timeout=10)
        (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(output / "evidence.json")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image-manifest", type=Path, required=True)
    parser.add_argument("--release", type=Path, required=True, help="existing Linux release; only OS/process checks")
    parser.add_argument("--output", type=Path, required=True, help="new private evidence directory")
    verify(parser.parse_args())
