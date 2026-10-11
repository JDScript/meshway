#!/usr/bin/env python3
"""Build once on the host; deploy immutable binaries to three persistent Nodes."""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import time
from dev import DevError, private, save

ROOT = Path(__file__).resolve().parents[1]
NODES = ("lax", "sha", "nyc")
LABEL = "io.meshway.node-lab"
TARGETS = {
    "aarch64": ("aarch64-unknown-linux-musl", "linux/arm64", 183),
    "x86_64": ("x86_64-unknown-linux-musl", "linux/amd64", 62),
}


def run(*args, capture=False, **kwargs):
    return subprocess.run(args, cwd=ROOT, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None, **kwargs).stdout


def lab_dir():
    common = Path(run("git", "rev-parse", "--git-common-dir", capture=True).strip())
    common = (ROOT / common).resolve()
    path = common.parent / ".meshway" / "node-lab"
    private(path.parent, directory=True, create=True)
    private(path, directory=True, create=True)
    return path


def docker_target():
    info = json.loads(run("docker", "info", "--format", "{{json .}}", capture=True))
    if info["OSType"] != "linux" or info["Architecture"] not in TARGETS:
        raise SystemExit("Expected a Linux arm64 or amd64 Docker engine.")
    return TARGETS[info["Architecture"]]


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_elf(path, machine):
    with path.open("rb") as stream:
        header = stream.read(20)
    if (header[:6] != b"\x7fELF\x02\x01" or len(header) != 20
            or struct.unpack_from("<H", header, 18)[0] != machine):
        raise SystemExit(f"Wrong Linux ELF architecture: {path.name}")


def source_digest():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    for directory in (ROOT / "crates", ROOT / "apps/meshway-daemon", ROOT / "apps/meshway-cli"):
        paths.extend(path for path in directory.rglob("*")
                     if path.is_file() and (path.suffix == ".rs" or path.name == "Cargo.toml"))
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0")
        digest.update(path.read_bytes() + b"\0")
    return digest.hexdigest()


def build(lab):
    if shutil.disk_usage(ROOT).free < 50 * 1024 ** 3:
        raise SystemExit("Linux build needs at least 50 GiB of free disk space.")
    target, platform, machine = docker_target()
    before = source_digest()
    revision = run("git", "rev-parse", "HEAD", capture=True).strip()
    dirty = bool(run("git", "status", "--porcelain", capture=True).strip())
    # This dedicated cache is independent of host workspace checks.
    cache = ROOT / "target/node-linux"
    env = dict(os.environ, CARGO_TARGET_DIR=str(cache), CARGO_INCREMENTAL="0")
    run("rustup", "target", "add", target)
    run("cargo", "zigbuild", "--target", target, "--release", "--locked",
        "-p", "meshway-daemon", "-p", "meshway-cli", "--bins", env=env)
    if source_digest() != before:
        raise SystemExit("Rust sources changed during the build; rebuild before deploying.")
    releases = lab / "releases"
    releases.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=releases, prefix=".building-") as temporary:
        directory = Path(temporary)
        hashes = {}
        for name in ("meshwayd", "meshway"):
            source = cache / target / "release" / name
            check_elf(source, machine)
            shutil.copy2(source, directory / name)
            hashes[name] = sha256(directory / name)
        manifest = {"revision": revision, "dirty": dirty, "sourceDigest": before,
                    "target": target, "platform": platform, "sha256": hashes}
        identity = hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
        release = releases / identity
        (directory / "build.json").write_text(json.dumps(manifest, indent=2) + "\n")
        if not release.exists():
            # The mounted directory contains public executables, not Node state.
            directory.chmod(0o755)
            directory.rename(release)
        else:
            for name, digest in hashes.items():
                if sha256(release / name) != digest:
                    raise SystemExit("Existing immutable release was modified.")
        selected = lab / "selected.tmp"
        selected.write_text(str(release) + "\n")
        selected.replace(lab / "selected")
    print(f"Built {target}: {release}")


def owned_resources(required_nodes=(), check_orphans=False):
    containers = {}
    volumes = {}
    resources = [("container", f"meshway-{node}", "service", node) for node in NODES]
    resources += [("volume", f"meshway-{node}-state", "volume", f"{node}-state") for node in NODES]
    resources += [("network", "meshway-node-lab", "network", "default")]
    for kind, name, label, role in resources:
        result = subprocess.run(["docker", kind, "inspect", name], capture_output=True, text=True, timeout=45)
        if result.returncode:
            # Docker errors other than a missing exact resource must stop here.
            missing = "no such" in result.stderr.lower() or f"network {name} not found" in result.stderr.lower()
            if not missing:
                raise SystemExit(result.stderr)
            if kind == "volume" and (role.removesuffix("-state") in containers
                                     or role.removesuffix("-state") in required_nodes):
                raise DevError(f"Existing Node state volume is missing: {name}; refusing to replace its identity.")
            continue
        value = json.loads(result.stdout)[0]
        labels = (value["Config"] if kind == "container" else value).get("Labels") or {}
        if (labels.get(LABEL) != "1" or labels.get("com.docker.compose.project") != "meshway-node-lab"
                or labels.get("com.docker.compose." + label) != role):
            raise SystemExit(f"Existing {kind} is not owned by this lab: {name}")
        if kind == "container":
            containers[role] = value
        elif kind == "volume":
            node = role.removesuffix("-state")
            volumes[node] = value
            if check_orphans and node not in containers:
                raise DevError(f"State volume has no inspectable Node or unfinished deploy plan: {name}; namespace/intent is unknown.")
    return containers, volumes


def selected_release(lab):
    release = Path((lab / "selected").read_text().strip()).resolve()
    if release.parent != (lab / "releases").resolve():
        raise SystemExit("Selected binary release is outside this lab.")
    manifest = json.loads((release / "build.json").read_text())
    target, platform, machine = docker_target()
    if manifest["target"] != target or manifest["platform"] != platform:
        raise SystemExit("Build architecture differs from the Docker engine; rebuild first.")
    for name in ("meshwayd", "meshway"):
        check_elf(release / name, machine)
        if sha256(release / name) != manifest["sha256"][name]:
            raise SystemExit(f"Built binary checksum differs: {name}")
    return release, manifest


def image_builder():
    spec = importlib.util.spec_from_file_location("node_lab_image", ROOT / "dev/node-lab/build.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def base_image():
    """Pinned Ubuntu base for explicit fixtures; no image build or selection."""
    image = json.loads((ROOT / "dev/node-lab/assets.lock.json").read_text())["base"]
    if not re.fullmatch(r"ubuntu:24\.04@sha256:[0-9a-f]{64}", image):
        raise DevError("Invalid pinned Ubuntu base image.")
    return image


def selected_image(lab, platform):
    """Read/verify an explicitly built local OS image; never builds or pulls."""
    selection = lab / "selected-image.json"
    private(selection)
    path = Path(json.loads(selection.read_text())["manifest"]).resolve()
    if path.name != "image-manifest.json" or path.parent.parent != (lab / "images").resolve():
        raise DevError("Selected OS image manifest is outside this lab.")
    private(path.parent, directory=True)
    manifest = json.loads(path.read_text())
    return verify_image(manifest, platform)


def verify_image(manifest, platform):
    """Validate image provenance before changing an existing selection."""
    if (manifest.get("status") != "built" or manifest.get("platform") != platform
            or manifest.get("sourceDigest") != image_builder().source_digest()
            or manifest.get("base") != base_image()
            or not manifest.get("packages") or not manifest.get("sources")
            or not re.fullmatch(r"sha256:[0-9a-f]{64}", manifest.get("imageId", ""))):
        raise DevError("Selected OS image inputs differ; explicitly run node:lab image-build.")
    image = json.loads(run("docker", "image", "inspect", manifest["imageId"], capture=True, timeout=45))[0]
    config = image["Config"]
    if (image["Id"] != manifest["imageId"] or image["Os"] + "/" + image["Architecture"] != platform
            or config.get("Volumes") or config.get("User") not in (None, "", "0", "root")
            or config.get("Labels", {}).get("io.meshway.node-lab.image") != "1"
            or config.get("Labels", {}).get("io.meshway.node-lab.source") != manifest["sourceDigest"]
            or config.get("Entrypoint") != ["/usr/local/lib/meshway-lab/entrypoint.sh"]):
        raise DevError("Selected OS image ownership/architecture/config differs.")
    return manifest


def build_image(lab):
    _, platform, _ = docker_target()
    builder = image_builder()
    output = lab / "images" / (builder.source_digest() + "-" + platform.split("/")[1])
    manifest = verify_image(builder.build(output, platform), platform)
    save(lab / "selected-image.json", {"manifest": str(output / "image-manifest.json")})
    print(f"Built OS image {manifest['imageId']}: {output / 'image-manifest.json'}")


def deployment_digest():
    digest = hashlib.sha256()
    for name in ("dev/node-lab/compose.yaml", "scripts/node_lab.py", "scripts/dev/lab.ts"):
        digest.update(name.encode() + b"\0" + (ROOT / name).read_bytes() + b"\0")
    return digest.hexdigest()


def compose(env, *args, capture=False):
    return run("docker", "compose", "-f", str(ROOT / "dev/node-lab/compose.yaml"),
               *args, env=env, capture=capture, timeout=120)


def config_hashes(env):
    hashes = {}
    for line in compose(env, "config", "--hash", "*", capture=True).splitlines():
        fields = line.split()
        if (len(fields) != 2 or fields[0] not in NODES or fields[0] in hashes
                or not re.fullmatch(r"[0-9a-f]{64}", fields[1])):
            raise DevError("Unexpected Compose config hash output.")
        hashes[fields[0]] = fields[1]
    if set(hashes) != set(NODES):
        raise DevError("Compose must contain exactly the three lab Node roles.")
    return hashes


def node_status(node, container):
    if not container["State"]["Running"]:
        raise DevError(f"meshway-{node}: existing container is stopped; cannot verify Node intent/namespace cleanup.")
    value = json.loads(run("docker", "exec", container["Id"], "/opt/meshway/meshway",
                           "status", "--json", capture=True, timeout=45))
    if (not isinstance(value, dict) or type(value.get("desiredUp")) is not bool
            or value.get("networkMode") not in ("tun", "controlOnly")
            or value.get("dataPlane") not in ("controlOnly", "stopped", "preparing", "ready", "suspended", "failed")
            or not isinstance(value.get("phase"), str) or "identity" not in value):
        raise DevError(f"meshway-{node}: invalid CLI status.")
    node_identity(value["identity"])
    return value


def daemon_proof(node, container, status):
    """Hash the IPC-reported live executable, never the PID-1 service owner."""
    pid = status.get("processId")
    if type(pid) is not int or not 1 <= pid <= 4194304:
        raise DevError(f"meshway-{node}: invalid daemon processId.")
    mounts = [mount for mount in container.get("Mounts", []) if mount.get("Destination") == "/opt/meshway"]
    if len(mounts) != 1 or mounts[0].get("Type") != "bind" or mounts[0].get("RW") is not False:
        raise DevError(f"meshway-{node}: daemon release must be a read-only bind.")
    script = (f'readlink /proc/{pid}/exe; sha256sum /proc/{pid}/exe; '
              f'cat /proc/{pid}/stat; grep "^Uid:" /proc/{pid}/status; '
              f'readlink /proc/{pid}/ns/mnt; readlink /proc/1/ns/mnt; cat /opt/meshway/build.json')
    lines = run("docker", "exec", container["Id"], "/bin/sh", "-ec", script, capture=True, timeout=45).splitlines()
    if len(lines) < 7:
        raise DevError(f"meshway-{node}: incomplete running daemon proof.")
    executable, checksum, process, uid, namespace, init_namespace = lines[:6]
    manifest = json.loads("\n".join(lines[6:]))
    fields = process.rsplit(") ", 1)[-1].split()
    digest = checksum.split()[0]
    if (executable != "/opt/meshway/meshwayd" or not re.fullmatch(r"[0-9a-f]{64}", digest)
            or digest != manifest.get("sha256", {}).get("meshwayd") or uid.split() != ["Uid:", "0", "0", "0", "0"]
            or namespace != init_namespace or len(fields) < 20 or not fields[19].isdigit()
            or not process.startswith(str(pid) + " ")):
        raise DevError(f"meshway-{node}: live daemon executable/UID/namespace differs from its release.")
    return {"processId": pid, "startTicks": int(fields[19]), "sha256": digest,
            "mountNamespace": namespace, "release": mounts[0]["Source"]}


def node_identity(value):
    if value is None:
        return None
    if (not isinstance(value, dict)
            or any(not isinstance(value.get(key), str) or not value[key] or len(value[key]) > 256
                   for key in ("nodeId", "networkId"))):
        raise DevError("Invalid Node identity.")
    return {key: value[key] for key in ("nodeId", "networkId")}


def volume_identity(node, volume, container=None):
    limits = {"Name": 256, "CreatedAt": 128, "Driver": 128, "Mountpoint": 4096, "Scope": 128}
    if (not isinstance(volume, dict) or volume.get("Name") != f"meshway-{node}-state"
            or any(not isinstance(volume.get(key), str) or not volume[key] or len(volume[key]) > limit for key, limit in limits.items())):
        raise DevError(f"meshway-{node}: state volume identity is unavailable.")
    if container is not None:
        mounts = [mount for mount in container.get("Mounts", []) if mount.get("Destination") == "/var/lib/meshway"]
        if (len(mounts) != 1 or mounts[0].get("Type") != "volume" or mounts[0].get("Name") != volume["Name"]
                or mounts[0].get("Source") != volume["Mountpoint"] or mounts[0].get("RW") is not True):
            raise DevError(f"meshway-{node}: state volume mount differs.")
    settings = json.dumps({key: volume.get(key) for key in ("Labels", "Options")}, sort_keys=True)
    return {**{key: volume[key] for key in limits}, "settingsSha256": hashlib.sha256(settings.encode()).hexdigest()}


def container_run(container):
    value = {"startedAt": container["State"].get("StartedAt"), "restartCount": container.get("RestartCount")}
    if (not isinstance(value["startedAt"], str) or not value["startedAt"] or len(value["startedAt"]) > 128
            or type(value["restartCount"]) is not int or value["restartCount"] < 0):
        raise DevError("Container run identity is unavailable.")
    return value


def stopped_receipt(container):
    state = container["State"]
    if (state["Running"] or state.get("Status") != "exited" or state.get("Paused") or state.get("Restarting")
            or state.get("Dead") or not isinstance(state.get("FinishedAt"), str)
            or not state["FinishedAt"] or len(state["FinishedAt"]) > 128):
        raise DevError("Container has not confirmed a managed stop.")
    return {**container_run(container), "finishedAt": state["FinishedAt"]}


def wait_node_status(node, container):
    deadline = time.monotonic() + 45
    while True:
        try:
            return node_status(node, container)
        except subprocess.CalledProcessError:
            if time.monotonic() >= deadline:
                raise DevError(f"meshway-{node}: restarted daemon IPC is still unavailable; journal retained.") from None
            time.sleep(.25)


def deploy_journal(lab):
    path = lab / "deploy.json"
    if not path.exists() and not path.is_symlink():
        return None
    private(path)
    if path.stat().st_size > 32768:
        raise DevError("Deploy journal exceeds its size bound; retained for inspection.")
    value = json.loads(path.read_text())
    if (not isinstance(value, dict) or value.get("version") != 1
            or value.get("phase") not in ("preparing", "composing", "restoring", "complete",
                                           "stop-preparing", "stop-stopping", "managed-stopped", "resuming")
            or not isinstance(value.get("plan"), dict)
            or not isinstance(value.get("nodes"), dict) or set(value["nodes"]) != set(NODES)):
        raise DevError("Invalid deploy journal; retained for inspection.")
    for entry in value["nodes"].values():
        if (not isinstance(entry, dict)
                or any(type(entry.get(key)) is not bool for key in ("desiredUp", "recreate", "downRequested", "quiesced"))
                or entry.get("networkMode") not in (None, "tun", "controlOnly")
                or any(entry.get(key) is not None and not isinstance(entry[key], str)
                       for key in ("containerId", "configHash", "deployedContainerId"))
                or "identity" not in entry):
            raise DevError("Invalid deploy Node record; retained for inspection.")
        node_identity(entry["identity"])
        if value["phase"] in ("stop-preparing", "stop-stopping", "managed-stopped", "resuming"):
            if (not isinstance(entry.get("stateVolume"), dict) or not isinstance(entry.get("stopOrigin"), dict)
                    or type(entry.get("stopRequested")) is not bool or type(entry.get("startRequested")) is not bool
                    or "stopReceipt" not in entry or "resumedRun" not in entry):
                raise DevError("Invalid managed-stop proof; journal retained.")
    return value


def snapshot(journal=None, *, allow_stopped=False):
    required = [node for node, entry in journal["nodes"].items() if entry["containerId"]] if journal else ()
    containers, volumes = owned_resources(required, check_orphans=journal is None)
    for node, volume in volumes.items():
        identity = volume_identity(node, volume, containers.get(node))
        if node in containers:
            containers[node]["stateVolume"] = identity
        recorded = journal["nodes"][node].get("stateVolume") if journal else None
        if recorded is not None and recorded != identity:
            raise DevError(f"meshway-{node}: state volume identity changed; journal retained.")
    statuses = {}
    for node, container in containers.items():
        if allow_stopped and not container["State"]["Running"]:
            statuses[node] = None
        elif journal and journal["phase"] == "resuming" and journal["nodes"][node]["startRequested"]:
            statuses[node] = wait_node_status(node, container)
        else:
            statuses[node] = node_status(node, container)
        if statuses[node] is not None:
            container["daemonProof"] = daemon_proof(node, container, statuses[node])
    return containers, statuses


CONVERGENCE_SECONDS = 60


def converged(status, desired_up):
    """The restored intent is fully applied (not merely saved or retrying)."""
    if status["desiredUp"] != desired_up:
        return False
    if not desired_up:
        return status["phase"] == "down" and status["dataPlane"] in ("stopped", "controlOnly")
    ready = status.get("authenticated") is True and status.get("lastError") is None
    if status["networkMode"] == "tun":
        return (ready and status["phase"] == "running" and status["dataPlane"] == "ready"
                and status.get("applied") is not None and status["applied"] == status.get("saved"))
    return ready and status["phase"] == "controlReady" and status["dataPlane"] == "controlOnly"


def node_records(containers, statuses, hashes):
    nodes = {}
    for node in NODES:
        container, status = containers.get(node), statuses.get(node)
        current_hash = container["Config"]["Labels"].get("com.docker.compose.config-hash") if container else None
        if container and not re.fullmatch(r"[0-9a-f]{64}", current_hash or ""):
            raise DevError(f"meshway-{node}: missing/invalid current Compose hash.")
        nodes[node] = {"containerId": container["Id"] if container else None, "configHash": current_hash,
                       "identity": node_identity(status["identity"]) if status else None,
                       "desiredUp": status["desiredUp"] if status else False,
                       "networkMode": status["networkMode"] if status else None,
                       "recreate": bool(container and current_hash != hashes[node]),
                       "downRequested": False, "quiesced": False, "deployedContainerId": None,
                       "stateVolume": container["stateVolume"] if container else None,
                       "daemonProof": container["daemonProof"] if container else None}
    return nodes


def check_managed_stop(journal, containers, statuses):
    if set(containers) != set(NODES):
        raise DevError("Managed stop container is missing; journal retained.")
    for node, entry in journal["nodes"].items():
        container, status = containers[node], statuses[node]
        if (container["Id"] != entry["containerId"]
                or container["Config"]["Labels"].get("com.docker.compose.config-hash") != entry["configHash"]):
            raise DevError(f"meshway-{node}: managed-stop container/config conflict; journal retained.")
        if not container["State"]["Running"]:
            receipt = stopped_receipt(container)
            if (not entry["quiesced"] or not entry["stopRequested"] or journal["phase"] == "stop-preparing"
                    or container_run(container) != entry["stopOrigin"]
                    or journal["phase"] in ("managed-stopped", "resuming") and entry["stopReceipt"] is None
                    or entry["stopReceipt"] is not None and receipt != entry["stopReceipt"]):
                raise DevError(f"meshway-{node}: stopped container lacks matching quiescence proof; journal retained.")
            continue
        if journal["phase"] == "managed-stopped":
            raise DevError(f"meshway-{node}: managed-stopped container was restarted outside this plan; journal retained.")
        if journal["phase"] == "resuming":
            if (not entry["startRequested"] or entry["stopReceipt"] is None
                    or entry["resumedRun"] is not None and container_run(container) != entry["resumedRun"]):
                raise DevError(f"meshway-{node}: container restart lacks managed-stop proof; journal retained.")
        elif container_run(container) != entry["stopOrigin"]:
            raise DevError(f"meshway-{node}: container run identity changed during stop; journal retained.")
        proof = container["daemonProof"]
        original = entry.get("daemonProof")
        if (original is not None and (proof["sha256"] != original["sha256"] or proof["release"] != original["release"])
                or journal["phase"] != "resuming" and original is not None and proof != original
                or entry.get("resumedDaemon") is not None and proof != entry["resumedDaemon"]):
            raise DevError(f"meshway-{node}: managed-stop daemon process/release conflict; journal retained.")
        allowed_up = {entry["desiredUp"]}
        if entry["networkMode"] == "tun" and entry["downRequested"]:
            allowed_up.add(False)
        if entry["networkMode"] == "tun" and entry["quiesced"]:
            allowed_up = {False}
        if (node_identity(status["identity"]) != entry["identity"] or status["networkMode"] != entry["networkMode"]
                or status["desiredUp"] not in allowed_up
                or entry["quiesced"] and (status["dataPlane"] != ("stopped" if entry["networkMode"] == "tun" else "controlOnly"))):
            raise DevError(f"meshway-{node}: managed-stop identity/intent/quiescence conflict; journal retained.")


def stop(lab):
    journal = deploy_journal(lab)
    if journal and journal["phase"] not in ("complete", "stop-preparing", "stop-stopping", "managed-stopped"):
        raise DevError("Finish the incomplete deploy before managed stop; journal retained.")
    if not journal or journal["phase"] == "complete":
        containers, statuses = snapshot()
        if set(containers) != set(NODES):
            raise DevError("Managed stop requires all three inspectable lab Nodes.")
        hashes = {node: container["Config"]["Labels"].get("com.docker.compose.config-hash") for node, container in containers.items()}
        nodes = node_records(containers, statuses, hashes)
        for node, entry in nodes.items():
            entry.update(stopOrigin=container_run(containers[node]), stopRequested=False,
                         stopReceipt=None, startRequested=False, resumedRun=None)
        journal = {"version": 1, "plan": {"root": str(ROOT), "hashes": hashes}, "nodes": nodes, "phase": "stop-preparing"}
        save(lab / "deploy.json", journal)
    containers, statuses = snapshot(journal, allow_stopped=True)
    check_managed_stop(journal, containers, statuses)
    if journal["phase"] == "stop-preparing":
        for node, entry in journal["nodes"].items():
            if entry["networkMode"] == "tun" and statuses[node]["desiredUp"]:
                entry["downRequested"] = True
                save(lab / "deploy.json", journal)
                run("docker", "exec", entry["containerId"], "/opt/meshway/meshway", "down", "--json", timeout=45)
            containers, statuses = snapshot(journal)
            check_managed_stop(journal, containers, statuses)
            expected_plane = "stopped" if entry["networkMode"] == "tun" else "controlOnly"
            if statuses[node]["dataPlane"] != expected_plane or entry["networkMode"] == "tun" and statuses[node]["desiredUp"]:
                raise DevError(f"meshway-{node}: managed stop did not confirm quiescence; journal retained.")
            entry["quiesced"] = True
            save(lab / "deploy.json", journal)
        journal["phase"] = "stop-stopping"
        save(lab / "deploy.json", journal)
    if journal["phase"] == "stop-stopping":
        for node, entry in journal["nodes"].items():
            containers, statuses = snapshot(journal, allow_stopped=True)
            check_managed_stop(journal, containers, statuses)
            if containers[node]["State"]["Running"]:
                entry["stopRequested"] = True
                save(lab / "deploy.json", journal)
                # Wait for graceful shutdown; Docker must not escalate to SIGKILL.
                run("docker", "stop", "--time", "-1", entry["containerId"], timeout=45)
                containers, statuses = snapshot(journal, allow_stopped=True)
                check_managed_stop(journal, containers, statuses)
            entry["stopReceipt"] = stopped_receipt(containers[node])
            save(lab / "deploy.json", journal)
        journal["phase"] = "managed-stopped"
        save(lab / "deploy.json", journal)
    print("Lab containers are managed-stopped; original Node identities and up intent retained for deploy.")


def resume_stopped(lab, journal):
    containers, statuses = snapshot(journal, allow_stopped=True)
    check_managed_stop(journal, containers, statuses)
    for node, entry in journal["nodes"].items():
        if not containers[node]["State"]["Running"]:
            entry["startRequested"] = True
            save(lab / "deploy.json", journal)
            run("docker", "start", entry["containerId"], timeout=45)
            containers, statuses = snapshot(journal, allow_stopped=True)
            check_managed_stop(journal, containers, statuses)
        if not containers[node]["State"]["Running"]:
            raise DevError(f"meshway-{node}: managed restart is incomplete; journal retained.")
        entry["resumedRun"] = container_run(containers[node])
        entry["resumedDaemon"] = containers[node]["daemonProof"]
        save(lab / "deploy.json", journal)
    journal["phase"] = "preparing"
    save(lab / "deploy.json", journal)


def check_snapshot(journal, containers, statuses, *, deployed=False):
    for node, entry in journal["nodes"].items():
        container = containers.get(node)
        if container is None:
            # Compose can fail after removing the old container but before creating
            # its replacement. The exact owned state volume must still exist.
            if (not deployed and journal["phase"] == "composing" and entry["recreate"]
                    and entry["deployedContainerId"] is None
                    and (entry["networkMode"] != "tun" or entry["quiesced"])):
                continue
            if entry["containerId"] is not None or deployed:
                raise DevError(f"meshway-{node}: planned container is missing; deploy journal retained.")
            continue
        status = statuses[node]
        original = container["Id"] == entry["containerId"]
        expected_hash = journal["plan"]["hashes"][node]
        current_hash = container["Config"]["Labels"].get("com.docker.compose.config-hash")
        if entry["identity"] is None and status["identity"] is not None and entry["containerId"] is not None:
            # A blocked original daemon reported no identity; the first identity
            # observed on the same state volume becomes the plan's identity.
            entry["identity"] = node_identity(status["identity"])
        if (node_identity(status["identity"]) != entry["identity"]
                or (entry["deployedContainerId"] is not None and container["Id"] != entry["deployedContainerId"])
                or (not original and journal["phase"] == "preparing")
                or (not original and entry["containerId"] is not None and not entry["recreate"])
                or current_hash != (expected_hash if deployed or not original else entry["configHash"])
                or original and status["networkMode"] != entry["networkMode"]
                or original and entry.get("resumedRun") is not None and container_run(container) != entry["resumedRun"]):
            raise DevError(f"meshway-{node}: deploy plan/container/identity conflict; journal retained.")
        proof = container["daemonProof"]
        expected_proof = entry.get("resumedDaemon") or entry.get("daemonProof")
        if (original and expected_proof is not None and proof != expected_proof
                or (deployed or not original or current_hash == expected_hash) and (proof["sha256"] != journal["plan"]["daemonSha256"]
                    or proof["release"] != journal["plan"]["release"]
                    or container.get("Image") != journal["plan"]["imageId"])):
            raise DevError(f"meshway-{node}: deploy daemon process/image/release conflict; journal retained.")
        if not entry["desiredUp"] and status["desiredUp"]:
            raise DevError(f"meshway-{node}: previously down Node intent changed; journal retained.")
        needs_down = entry["recreate"] and entry["networkMode"] == "tun"
        if needs_down and journal["phase"] in ("composing", "restoring") and original:
            if not entry["quiesced"] or status["desiredUp"] or not (status["dataPlane"] == "stopped"
                                                                      or underlay_blocked(status)):
                raise DevError(f"meshway-{node}: old TUN namespace is not stopped; journal retained.")
        if journal["phase"] == "preparing":
            allowed_down = (needs_down or entry.get("stopReceipt") is not None) and entry["downRequested"]
            if status["desiredUp"] != entry["desiredUp"] and not allowed_down:
                raise DevError(f"meshway-{node}: original Node intent changed; journal retained.")


def underlay_blocked(status):
    """A Node whose underlay never started in this process owns no kernel
    resources in its namespace; recreating the container destroys that
    namespace and the new daemon still judges the journal itself."""
    return (not status["desiredUp"] and status["phase"] == "blocked"
            and status["dataPlane"] == "failed" and status.get("lastError") == "underlay")


def deploy(lab):
    release, manifest = selected_release(lab)
    image = selected_image(lab, manifest["platform"])
    env = dict(os.environ, MESHWAY_NODE_RELEASE=str(release), MESHWAY_NODE_PLATFORM=manifest["platform"],
               MESHWAY_NODE_IMAGE=image["imageId"])
    plan = {"root": str(ROOT), "release": str(release), "platform": manifest["platform"], "hashes": config_hashes(env),
            "imageId": image["imageId"], "imageSourceDigest": image["sourceDigest"],
            "sourceDigest": deployment_digest(), "daemonSha256": manifest["sha256"]["meshwayd"]}
    journal = deploy_journal(lab)
    if journal and journal["phase"] in ("stop-preparing", "stop-stopping"):
        raise DevError("Finish the incomplete managed stop before deploy; journal retained.")
    if journal and journal["phase"] == "managed-stopped":
        containers, statuses = snapshot(journal, allow_stopped=True)
        check_managed_stop(journal, containers, statuses)
        for node, entry in journal["nodes"].items():
            entry["recreate"] = entry["configHash"] != plan["hashes"][node]
        journal.update(plan=plan, phase="resuming")
        save(lab / "deploy.json", journal)
    if journal and journal["phase"] != "complete" and journal["plan"] != plan:
        raise DevError("Incomplete deploy has a different release/config plan; journal retained. Retry its original plan.")
    if journal and journal["phase"] == "resuming":
        resume_stopped(lab, journal)
    containers, statuses = snapshot(journal if journal and journal["phase"] != "complete" else None)
    if not journal or journal["phase"] == "complete":
        nodes = node_records(containers, statuses, plan["hashes"])
        journal = {"version": 1, "plan": plan, "nodes": nodes, "phase": "preparing"}
        save(lab / "deploy.json", journal)
    check_snapshot(journal, containers, statuses, deployed=journal["phase"] == "restoring")
    if journal["phase"] == "preparing":
        for node, entry in journal["nodes"].items():
            if not entry["recreate"] or entry["networkMode"] != "tun":
                continue
            if statuses[node]["desiredUp"]:
                entry["downRequested"] = True
                save(lab / "deploy.json", journal)
                try:
                    run("docker", "exec", entry["containerId"], "/opt/meshway/meshway", "down", "--json", timeout=45)
                except subprocess.CalledProcessError:
                    # The CLI exits nonzero when the intent is committed but the
                    # Node still reports a problem; only a blocked underlay that
                    # never installed anything is accepted below.
                    containers, statuses = snapshot(journal)
                    if not underlay_blocked(statuses[node]):
                        raise
            containers, statuses = snapshot(journal)
            check_snapshot(journal, containers, statuses)
            if statuses[node]["desiredUp"] or not (statuses[node]["dataPlane"] == "stopped"
                                                    or underlay_blocked(statuses[node])):
                raise DevError(f"meshway-{node}: down did not confirm stopped data plane; journal retained.")
            entry["quiesced"] = True
            save(lab / "deploy.json", journal)
        journal["phase"] = "composing"
        save(lab / "deploy.json", journal)
    if journal["phase"] == "composing":
        containers, statuses = snapshot(journal)
        check_snapshot(journal, containers, statuses)
        if (config_hashes(env) != plan["hashes"] or deployment_digest() != plan["sourceDigest"]
                or selected_image(lab, manifest["platform"]) != image):
            raise DevError("Compose config changed during deploy; original plan journal retained.")
        changed = [node for node in NODES if node not in containers
                   or containers[node]["Config"]["Labels"]["com.docker.compose.config-hash"] != plan["hashes"][node]]
        if changed:
            # Exclude matching containers even if the source file changes between
            # hash inspection and Compose: only these namespaces were prepared.
            compose(env, "up", "-d", "--no-build", "--no-deps", "--wait", "--wait-timeout", "60", *changed)
        containers, statuses = snapshot(journal)
        check_snapshot(journal, containers, statuses, deployed=True)
        for node, container in containers.items():
            journal["nodes"][node]["deployedContainerId"] = container["Id"]
        journal["phase"] = "restoring"
        save(lab / "deploy.json", journal)
    for node, entry in journal["nodes"].items():
        if entry["desiredUp"] and not statuses[node]["desiredUp"]:
            run("docker", "exec", containers[node]["Id"], "/opt/meshway/meshway", "up", "--json", timeout=45)
            containers, statuses = snapshot(journal)
            check_snapshot(journal, containers, statuses, deployed=True)
    # A restored intent converges through transient retries (the CLI exits 0
    # for them too); only a blocked Node or a deadline without convergence fails.
    deadline = time.monotonic() + CONVERGENCE_SECONDS
    while True:
        containers, statuses = snapshot(journal)
        check_snapshot(journal, containers, statuses, deployed=True)
        pending = [f"meshway-{node}" for node, status in statuses.items()
                   if not converged(status, journal["nodes"][node]["desiredUp"])]
        blocked = any(status["phase"] == "blocked" for status in statuses.values())
        if not pending or blocked or time.monotonic() >= deadline:
            break
        time.sleep(1)
    for node, status in statuses.items():
        print(json.dumps({"container": f"meshway-{node}", "status": status}), flush=True)
    if pending:
        raise DevError("Deploy control/install or intent restoration remains pending or failed: " + ", ".join(pending) + "; journal retained; retry the same deploy.")
    journal["phase"] = "complete"
    save(lab / "deploy.json", journal)
    print("Node deploy complete; actual states reported above.")


def configure(lab):
    journal = deploy_journal(lab)
    if journal and journal["phase"] not in ("restoring", "complete"):
        raise DevError("Finish the incomplete deploy before configuring the lab.")
    containers, statuses = snapshot(journal if journal and journal["phase"] == "restoring" else None)
    if set(containers) != set(NODES):
        raise DevError("Configure requires all three deployed lab Nodes.")
    if journal and journal["phase"] == "restoring":
        check_snapshot(journal, containers, statuses, deployed=True)
    helper = ROOT / "scripts/dev/lab.ts"
    if not helper.is_file():
        raise DevError("Missing lab configuration helper.")
    run("node", "--experimental-strip-types", str(helper), str(ROOT),
        str(lab.parent / "dev-login.json"), *(f"meshway-{node}" for node in NODES), timeout=180)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["build", "image-build", "deploy", "configure", "versions", "status", "stop"])
    args = parser.parse_args(argv)
    lab = lab_dir()
    if args.action in ("versions", "status"):
        operate(args.action, lab)
        return
    path = lab / "operation.lock"
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        info = os.fstat(descriptor)
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1
                or stat.S_IMODE(info.st_mode) not in (0o600, 0o644)):
            raise DevError("Invalid lab operation lock ownership or permissions.")
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        # Older lab versions created this non-secret lock with the default umask.
        os.fchmod(descriptor, 0o600)
        private(path)
        operate(args.action, lab)
    finally:
        os.close(descriptor)


def operate(action, lab):
    if action == "build":
        build(lab)
        return
    if action == "image-build":
        build_image(lab)
        return
    owned_resources()
    if action in ("versions", "status"):
        for node in NODES:
            name = f"meshway-{node}"
            print(f"{name}:", flush=True)
            state = json.loads(run("docker", "inspect", name, "--format", "{{json .State}}", capture=True))
            if not state["Running"]:
                print(json.dumps({"state": state["Status"]}))
                continue
            if action == "status":
                run("docker", "exec", name, "/opt/meshway/meshway", "status", "--json")
            else:
                manifest = json.loads(run("docker", "exec", name, "cat", "/opt/meshway/build.json", capture=True))
                container = json.loads(run("docker", "container", "inspect", name, capture=True, timeout=45))[0]
                proof = daemon_proof(node, container, node_status(node, container))
                print(json.dumps({**manifest, "runningDaemonMatches": True, "daemonProof": proof,
                                  "imageId": container["Image"]}, indent=2))
        return
    if action == "stop":
        stop(lab)
        return
    if action == "configure":
        configure(lab)
        return
    deploy(lab)


if __name__ == "__main__":
    try:
        main()
    except (DevError, OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"Node lab operation failed: {error}; any incomplete deploy journal is retained.", file=sys.stderr)
        raise SystemExit(1)
