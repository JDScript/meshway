#!/usr/bin/env python3
"""Build once on the host; deploy immutable binaries to three persistent Nodes."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

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
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
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


def owned_resources():
    resources = [("container", f"meshway-{node}") for node in NODES]
    resources += [("volume", f"meshway-{node}-state") for node in NODES]
    resources += [("network", "meshway-node-lab")]
    for kind, name in resources:
        result = subprocess.run(["docker", kind, "inspect", name], capture_output=True, text=True)
        if result.returncode:
            # Docker errors other than a missing exact resource must stop here.
            missing = "no such" in result.stderr.lower() or f"network {name} not found" in result.stderr.lower()
            if not missing:
                raise SystemExit(result.stderr)
            continue
        value = json.loads(result.stdout)[0]
        labels = (value["Config"] if kind == "container" else value).get("Labels") or {}
        if labels.get(LABEL) != "1" or labels.get("com.docker.compose.project") != "meshway-node-lab":
            raise SystemExit(f"Existing {kind} is not owned by this lab: {name}")


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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["build", "deploy", "versions", "status", "stop"])
    args = parser.parse_args()
    lab = lab_dir()
    if args.action in ("versions", "status"):
        operate(args.action, lab)
        return
    with (lab / "operation.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        operate(args.action, lab)


def operate(action, lab):
    if action == "build":
        build(lab)
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
                running = run("docker", "exec", name, "sha256sum", "/proc/1/exe", capture=True).split()[0]
                print(json.dumps({**manifest, "runningDaemonMatches": running == manifest["sha256"]["meshwayd"]}, indent=2))
        return
    if action == "stop":
        run("docker", "stop", "--time", "15", *(f"meshway-{node}" for node in NODES))
        return
    release, manifest = selected_release(lab)
    env = dict(os.environ, MESHWAY_NODE_RELEASE=str(release), MESHWAY_NODE_PLATFORM=manifest["platform"])
    run("docker", "compose", "-f", str(ROOT / "dev/node-lab/compose.yaml"),
        "up", "-d", "--no-build", "--wait", "--wait-timeout", "60", env=env)


if __name__ == "__main__":
    main()
