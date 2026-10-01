"""Writable demo binary deployment. Never recreate an existing Agent container."""
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import sys
import time
import uuid

from dev import DevError, LOCK, performance_environment, private, run, save

ROLES = ("nyc", "lax", "sha")
BIN = "/usr/local/bin/meshway-agent"
ARTIFACT_PARENT = Path(LOCK).parent / "meshway-dev-artifacts"


def require(condition, message):
    if not condition:
        raise DevError("demo update: " + message)


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def elf(path, architecture):
    with path.open("rb") as source:
        header = source.read(64)
    machine = {"amd64": 62, "arm64": 183}.get(architecture)
    require(machine is not None and len(header) == 64 and header[:6] == b"\x7fELF\x02\x01"
            and struct.unpack_from("<H", header, 18)[0] == machine
            and struct.unpack_from("<H", header, 16)[0] in (2, 3), "binary is not a native 64-bit Linux executable")


def material(data):
    """Hash immutable identity/key/profile material, not transient daemon runtime."""
    result = {}
    for role in ROLES:
        directory = data / role
        private(directory, directory=True)
        for path in directory.iterdir():
            if path.name in ("agent.json", "agent.json.connection-key.json", "profiles.json") or path.suffix == ".pem":
                private(path)
                result[str(path.relative_to(data))] = digest(path)
        require(all(role + "/" + name in result for name in ("agent.json", "agent.json.connection-key.json", "profiles.json")), "immutable material is missing")
    return result


def ownership(value, meta, data, role, running=True):
    """Fence Docker labels, base pin, startup identity and state binding together."""
    config, host = value["Config"], value["HostConfig"]
    expected = {"io.meshway.demo.project": "managed-neighbors", "io.meshway.demo.run": meta["run_id"],
                "io.meshway.demo.data": meta["fingerprint"], "io.meshway.demo.resource": role}
    require(all(config.get("Labels", {}).get(key) == val for key, val in expected.items()), "foreign container labels")
    require(value["Image"] == meta["image_id"] and (not running or value["State"]["Running"] is True), "base pin differs or Agent is not running")
    require(host.get("ReadonlyRootfs") is False, "read-only demo cannot use in-place update; use the explicit demo.py upgrade-image flow (recreates containers)")
    require(host.get("Privileged") is False and host.get("NetworkMode") not in ("host", "none", "default", "bridge")
            and not str(host.get("NetworkMode")).startswith("container:"), "unsafe container network/privileges")
    require(config.get("Entrypoint") == ["setpriv"] and config.get("StopSignal") == "SIGINT" and config.get("User") == "0:0", "Agent bootstrap differs")
    env = config.get("Env", [])
    require(sorted(item for item in env if item.startswith(("MESHWAY_AGENT_UID=", "MESHWAY_AGENT_GID="))) == sorted(["MESHWAY_AGENT_UID=" + str(meta["uid"]), "MESHWAY_AGENT_GID=" + str(meta["gid"])]), "recorded Agent user differs")
    cmd = config.get("Cmd", [])
    packet = meta.get("mode", "neighbors") == "linux-ipv4-tun"
    require(meta.get("mode", "neighbors") in ("neighbors", "linux-ipv4-tun"), "unknown packet mode")
    capability = "-all,+net_admin" if packet else "-all"
    bootstrap = ["--reuid", str(meta["uid"]), "--regid", str(meta["gid"]), "--clear-groups", "--bounding-set=" + capability,
                 "--inh-caps=" + capability, "--ambient-caps=" + capability, "--no-new-privs", "meshway-agent"]
    arguments = ["run", *(["--apply-dataplane", "--tun-name", "meshway0", "--tun-mtu", "1000"] if packet else ["--apply-connections"]),
                 "--interval-seconds", "2", "--connection-lease-seconds", "15", "--connection-profiles", "/state/profiles.json", "--state", "/state/agent.json"]
    require(cmd == bootstrap + arguments, "Agent command differs")
    caps = lambda values: sorted(item.removeprefix("CAP_") for item in (values or []))
    require(caps(host.get("CapAdd")) == ["NET_ADMIN"] and caps(host.get("CapDrop")) == [] and not host.get("Init"), "developer capabilities differ")
    require(host.get("SecurityOpt") in (["no-new-privileges"], ["no-new-privileges=true"]), "security settings differ")
    require(host.get("Devices", []) == ([{"PathOnHost": "/dev/net/tun", "PathInContainer": "/dev/net/tun", "CgroupPermissions": "rw"}] if packet else [])
            and not host.get("DeviceRequests") and not host.get("DeviceCgroupRules"), "device bindings differ")
    mounts = value.get("Mounts", [])
    require(any(m.get("Type") == "bind" and m.get("Source") == str(data / role) and m.get("Destination") == "/state" and m.get("RW") is True for m in mounts), "state bind differs")
    require(all(m.get("Type") == "bind" and m.get("Destination") == "/state" or m.get("Type") == "tmpfs" and m.get("Destination") == "/tmp" for m in mounts), "unexpected persistent mount")
    return value["Id"]


class Docker:
    def __init__(self, root, descriptor):
        self.root, self.descriptor = root, descriptor

    def __call__(self, *args, timeout=40, check=True):
        return self.execute(*args, timeout=timeout, check=check).stdout.strip()

    def execute(self, *args, timeout=40, check=True):
        return run(["docker", *args], cwd=self.root, timeout=timeout, check=check,
                   pass_fds=(self.descriptor,))

    def inspect(self, name):
        values = json.loads(self("inspect", name))
        require(isinstance(values, list) and len(values) == 1, "unexpected Docker inspect response")
        return values[0]


def cleanup(docker, extraction, run_id, source):
    value = docker.inspect(extraction)
    require(value["Config"].get("Labels", {}).get("io.meshway.binary-update") == run_id
            and value["Image"] == source and value["State"]["Running"] is False
            and value["HostConfig"].get("NetworkMode") == "none", "extraction ownership changed; cleanup refused")
    docker("rm", value["Id"])


def cleanup_artifact(directory, inode, binary_inode):
    private(directory, directory=True)
    require(directory.lstat().st_ino == inode, "temporary artifact directory changed; cleanup refused")
    children = list(directory.iterdir())
    binary = directory / "meshway-agent"
    require(not children or children == [binary], "unexpected temporary artifacts; cleanup refused")
    if children:
        private(binary)
        require(binary_inode is not None and binary.lstat().st_ino == binary_inode, "temporary binary changed; cleanup refused")
        binary.unlink()
    directory.rmdir()


def update(demo, image=None):
    meta = demo.metadata()
    require(meta.get("shell", "readonly") == "development", "read-only demo cannot use in-place update; use demo.py upgrade-image explicitly (recreates containers)")
    require(meta.get("phase") == "ready", "demo enrollment is incomplete")
    require(not any((demo.data / name).exists() for name in ("upgrade.json", "mode-transition.json", "mode.json")), "pending demo transition requires existing recovery flow")
    # Reuse the shared runner implementation, with its FD inherited by EVERY Docker child.
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(demo.root / "experiments/transport"))
    from performance_lock import performance_lock
    os.environ["MESHWAY_PERF_LOCK"] = performance_environment()["MESHWAY_PERF_LOCK"]
    with performance_lock() as descriptor:
        return deploy(demo, meta, image, Docker(demo.root, descriptor))


def inventory(demo, meta, docker, running=True):
    containers = {}
    for role in ROLES:
        labels = {"io.meshway.demo.project": "managed-neighbors", "io.meshway.demo.run": meta["run_id"], "io.meshway.demo.data": meta["fingerprint"], "io.meshway.demo.resource": role}
        args = ["ps", "--all", "--quiet"]
        for key, value in labels.items():
            args += ["--filter", "label=" + key + "=" + value]
        matches = docker(*args).splitlines()
        require(len(matches) == 1, "owned Agent container missing or ambiguous: " + role)
        value = docker.inspect(matches[0])
        container_id = ownership(value, meta, demo.data, role, running=running)
        containers[role] = {"container_id": container_id, "started_before": value["State"]["StartedAt"], "updated": False, "verified": False}
    return containers


def graceful_stop(docker, demo, meta, role, container):
    value = docker.inspect(container)
    require(ownership(value, meta, demo.data, role, running=False) == container, "container changed before graceful stop")
    if not value["State"]["Running"]:
        return
    # Docker stop/restart escalates to SIGKILL on timeout. Signal and poll instead.
    docker("kill", "--signal", "SIGINT", container)
    deadline = time.monotonic() + 40
    while True:
        value = docker.inspect(container)
        require(ownership(value, meta, demo.data, role, running=False) == container, "container changed while stopping")
        if not value["State"]["Running"]:
            return
        require(time.monotonic() < deadline, "graceful SIGINT deadline exceeded; container retained: " + role)
        time.sleep(.25)


def lifecycle(demo, action):
    """Reuse initialized writable containers; no enrollment, configuration or recreation."""
    meta = demo.metadata()
    require(meta.get("shell") == "development" and meta.get("phase") == "ready", "writable lifecycle needs an initialized development demo")
    require(not any((demo.data / name).exists() for name in ("upgrade.json", "mode.json")), "pending demo transition requires existing recovery flow")
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(demo.root / "experiments/transport"))
    from performance_lock import performance_lock
    os.environ["MESHWAY_PERF_LOCK"] = performance_environment()["MESHWAY_PERF_LOCK"]
    with performance_lock() as descriptor:
        docker = Docker(demo.root, descriptor)
        before = material(demo.data)
        meta_before = digest(demo.data / "demo.json")
        containers = inventory(demo, meta, docker, running=False)
        if action in ("stop", "restart"):
            for role in reversed(ROLES):
                graceful_stop(docker, demo, meta, role, containers[role]["container_id"])
        if action in ("start", "restart"):
            for role in ("lax", "nyc", "sha"):
                container = containers[role]["container_id"]
                value = docker.inspect(container)
                require(ownership(value, meta, demo.data, role, running=False) == container, "container changed before start")
                if not value["State"]["Running"]:
                    docker("start", container)
        require(material(demo.data) == before and digest(demo.data / "demo.json") == meta_before, "immutable material changed during lifecycle")


def deploy(demo, meta, image, docker):
    run_id = uuid.uuid4().hex
    if image is None:
        image = "meshway-managedtest:dev-entry-" + run_id[:12]
        print("demo: building native managed Ubuntu image under shared performance lock…", flush=True)
        docker("build", "--file", str(demo.root / "experiments/managed-connections/Dockerfile.dev"), "--tag", image, str(demo.root), timeout=None)
    require(image.startswith("meshway-managedtest:") or re.fullmatch(r"sha256:[0-9a-f]{64}", image), "dedicated managed image reference required")
    source = json.loads(docker("image", "inspect", image))[0]
    architecture = docker("info", "--format", "{{.Architecture}}")
    architecture = {"aarch64": "arm64", "x86_64": "amd64"}.get(architecture, architecture)
    require(source.get("Os") == "linux" and source.get("Architecture") == architecture, "source image must be native Linux architecture")
    require(not source.get("Config", {}).get("Volumes"), "source image declares unexpected state volumes")
    source_id = source["Id"]
    before = material(demo.data)
    metadata_digest = digest(demo.data / "demo.json")
    # Discover by full ownership labels, not predictable container names.
    containers = inventory(demo, meta, docker)
    directory = demo.root / ".meshway/services" / ("binary-update-" + run_id[:12])
    private(directory.parent, directory=True)
    private(directory, directory=True, create=True)
    journal = directory / "deployment.json"
    report = {"version": 1, "status": "PREPARING", "source_image": source_id, "source_reference": image,
              "base_image": meta["image_id"], "preserved_writable_layer": True,
              "source_commit": run(["git", "rev-parse", "HEAD"], cwd=demo.root).stdout.strip(),
              "created_at": time.time(), "containers": containers, "material_before": before}
    save(journal, report)
    extraction = "meshway-binary-source-" + run_id[:12]
    extraction_id = None
    staged = set()
    staged_hashes = {}
    next_path = BIN + ".next-" + run_id
    artifact = None
    artifact_inode = binary_inode = None
    try:
        for role, record in containers.items():
            container = record["container_id"]
            previous = directory / (role + ".previous")
            docker("cp", container + ":" + BIN, str(previous))
            previous.chmod(0o600)
            private(previous)
            elf(previous, architecture)
            record["previous_sha256"] = digest(previous)
            record["packages_before"] = hashlib.sha256(docker("exec", container, "dpkg-query", "-W", "-f", "${Package} ${Version}\n").encode()).hexdigest()
            record["alias_before"] = docker("exec", container, "sha256sum", "/usr/local/bin/meshway").split()[0]
        extraction_id = docker("create", "--name", extraction, "--label", "io.meshway.binary-update=" + run_id,
                               "--network", "none", "--entrypoint", "/bin/true", source_id)
        private(ARTIFACT_PARENT, directory=True, create=True)
        artifact = ARTIFACT_PARENT / ("meshway-dev-binary-" + run_id)
        private(artifact, directory=True, create=True)
        artifact_inode = artifact.lstat().st_ino
        binary = artifact / "meshway-agent"
        docker("cp", extraction_id + ":" + BIN, str(binary))
        binary.chmod(0o600)
        private(binary)
        binary_inode = binary.lstat().st_ino
        elf(binary, architecture)
        expected = digest(binary)
        report["binary_sha256"] = expected
        report["status"] = "STAGING"
        save(journal, report)
        # Stage and validate every new binary before changing any running daemon.
        for role, record in containers.items():
            container = record["container_id"]
            require(ownership(docker.inspect(container), meta, demo.data, role) == container, "container changed before staging")
            require(docker("exec", container, "test", "!", "-e", next_path) == "", "staging path already exists")
            docker("exec", container, "test", "!", "-L", next_path)
            docker("cp", str(binary), container + ":" + next_path)
            staged.add(container)
            staged_hashes[container] = docker("exec", container, "sha256sum", next_path).split()[0]
            docker("exec", container, "chmod", "755", next_path)
            require(staged_hashes[container] == expected, "staged binary digest differs")
            require(docker("exec", container, "sha256sum", BIN).split()[0] == record["previous_sha256"], "current binary changed during deployment")
            require("stun-server" in docker("exec", container, next_path, "netcheck", "--help"), "new binary lacks expected diagnostics")
            mode_flag = "--apply-dataplane" if meta.get("mode") == "linux-ipv4-tun" else "--apply-connections"
            require(mode_flag in docker("exec", container, next_path, "run", "--help"), "new binary lacks required managed runtime mode")
        require(material(demo.data) == before and digest(demo.data / "demo.json") == metadata_digest, "private state changed before replacement")
        report["status"] = "UPDATING"
        save(journal, report)
        for role, record in containers.items():
            container = record["container_id"]
            require(ownership(docker.inspect(container), meta, demo.data, role) == container, "container changed before replacement")
            require(docker("exec", container, "sha256sum", BIN).split()[0] == record["previous_sha256"], "old binary changed before replacement")
            require(docker("exec", container, "sha256sum", next_path).split()[0] == expected, "staged binary changed before replacement")
            # Write the intention before atomic rename. Never restore a SQLite/state snapshot.
            record["replacement_started"] = True
            save(journal, report)
            docker("exec", container, "mv", "--", next_path, BIN)
            staged.remove(container)
            record["updated"] = True
            save(journal, report)
            graceful_stop(docker, demo, meta, role, container)
            docker("start", container)
            deadline = time.monotonic() + 40
            while True:
                output = docker.execute("exec", container, "meshway", "status", "--live", "--json", timeout=15, check=False)
                try:
                    value = json.loads(output.stdout)
                    outcome = value.get("outcome") if isinstance(value, dict) else None
                    ready = output.returncode == 0 and isinstance(outcome, dict) and outcome.get("status") == "ok"
                except ValueError:
                    ready = False
                if ready:
                    break
                require(time.monotonic() < deadline, "new daemon live CLI deadline exceeded: " + role)
                time.sleep(.5)
            # CLI outcome proves live IPC only; no claim of TUN/neighbor packet health here.
            value = docker.inspect(container)
            require(ownership(value, meta, demo.data, role) == container and value["State"]["StartedAt"] != record["started_before"], "same-container restart not proven")
            require(docker("exec", container, "sha256sum", BIN).split()[0] == expected, "installed binary digest differs")
            packages = hashlib.sha256(docker("exec", container, "dpkg-query", "-W", "-f", "${Package} ${Version}\n").encode()).hexdigest()
            require(packages == record["packages_before"], "installed apt packages changed")
            require(docker("exec", container, "sha256sum", "/usr/local/bin/meshway").split()[0] == record["alias_before"], "CLI alias changed")
            require(material(demo.data) == before and digest(demo.data / "demo.json") == metadata_digest, "immutable identity/key/profile metadata changed")
            record["verified"] = True
            record["live_ipc"] = True
            save(journal, report)
            print(role + ": binary updated; same container and writable layer retained", flush=True)
        report["status"] = "PASS"
        save(journal, report)
        print("demo deployment receipt: " + str(journal), flush=True)
    except BaseException:
        report["status"] = "FAILED_REQUIRES_INSPECTION"
        save(journal, report)
        raise
    finally:
        try:
            for container in staged:
                role = next(role for role, record in containers.items() if record["container_id"] == container)
                require(ownership(docker.inspect(container), meta, demo.data, role) == container, "staged artifact owner changed; cleanup refused")
                docker("exec", container, "test", "!", "-L", next_path)
                require(container in staged_hashes and docker("exec", container, "sha256sum", next_path).split()[0] == staged_hashes[container], "staged artifact changed; cleanup refused")
                docker("exec", container, "rm", "--", next_path)
            if extraction_id:
                cleanup(docker, extraction_id, run_id, source_id)
            if artifact is not None:
                cleanup_artifact(artifact, artifact_inode, binary_inode)
        except BaseException:
            report["status"] = "FAILED_CLEANUP_REQUIRES_INSPECTION"
            save(journal, report)
            raise
