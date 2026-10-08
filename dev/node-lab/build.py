#!/usr/bin/env python3
"""Build the locked Ubuntu userspace, independently of Node binary releases."""
import argparse
import hashlib
import importlib.util
import json
import lzma
import os
from pathlib import Path
import re
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SOURCE_FILES = [HERE / name for name in ("assets.lock.json", "Dockerfile", "build.py", "entrypoint.sh", "runtime.py")]
SOURCE_FILES.append(ROOT / "dev/dns-fixture/build.py")


# Rootless or multi-engine hosts select the Docker context explicitly; the
# CLI default is kept so a desktop machine never builds through a stray context.
# buildx names each context's built-in docker-driver builder after the context.
CONTEXT = os.environ.get("DOCKER_CONTEXT", "default")


def source_digest():
    digest = hashlib.sha256()
    for path in sorted(SOURCE_FILES):
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def build(output, platform):
    if shutil.disk_usage(ROOT).free < 50 * 1024 ** 3:
        raise RuntimeError("Image build needs at least 50 GiB of free disk space.")
    # Reuse the owner of package parsing, dependency closure and SHA downloads.
    spec = importlib.util.spec_from_file_location("locked_os_assets", ROOT / "dev/dns-fixture/build.py")
    assets = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(assets)
    before = source_digest()
    lock = json.loads((HERE / "assets.lock.json").read_text())
    arch = platform.split("/")[1]
    info = json.loads(subprocess.check_output(["docker", "info", "--format", "{{json .}}"], text=True))
    if info["OSType"] != "linux" or {"aarch64": "arm64", "x86_64": "amd64"}.get(info["Architecture"]) != arch:
        raise RuntimeError("Image build requires the native Linux Docker architecture.")
    default_info = json.loads(subprocess.check_output(
        ["docker", "--context", CONTEXT, "info", "--format", "{{json .}}"], text=True, timeout=45))
    if default_info["ID"] != info["ID"]:
        raise RuntimeError("The default builder must use the selected Docker engine.")
    builder = subprocess.check_output(["docker", "--context", CONTEXT, "buildx", "inspect", CONTEXT], text=True, timeout=45)
    if not re.search(r"^Driver:\s+docker$", builder, re.MULTILINE):
        raise RuntimeError("Image build requires the ordinary default Docker builder.")
    output.mkdir(parents=True, exist_ok=True, mode=0o700)
    if output.is_symlink() or output.stat().st_uid != os.getuid() or output.stat().st_mode & 0o077:
        raise RuntimeError("Image build output must be a private directory owned by the current user.")
    manifest = output / "image-manifest.json"
    if manifest.exists():
        previous = json.loads(manifest.read_text())
        if previous["sourceDigest"] != before or previous["platform"] != platform:
            raise RuntimeError("Build directory belongs to different image inputs.")
    else:
        manifest.write_text(json.dumps({"sourceDigest": before, "platform": platform, "status": "preparing"}) + "\n")
    index = {}
    for repo in lock["repositories"]:
        base = lock["snapshot"] + "/dists/" + repo["suite"]
        assets.fetch(base + "/Release", repo["sha256"])
        for component, sha in repo["indexes"][arch].items():
            path = output / (sha + ".xz")
            if not path.exists():
                path.write_bytes(assets.fetch(base + f"/{component}/binary-{arch}/Packages.xz", sha))
            data = path.read_bytes()
            if assets.digest(data) != sha:
                raise RuntimeError("Cached package index digest mismatch.")
            index.update(assets.records(lzma.decompress(data).decode()))
    selected = assets.closure(index, lock["packages"])
    packages = [{"name": item["Package"], "version": item["Version"], "architecture": item["Architecture"],
                 "filename": item["Filename"], "sha256": item["SHA256"]} for item in selected]
    context = output / "context"
    debs = context / "debs"
    debs.mkdir(parents=True, exist_ok=True)
    expected = {Path(item["filename"]).name for item in packages}
    if any(path.name not in expected for path in debs.iterdir()):
        raise RuntimeError("Unexpected package in this locked build context.")
    for item in packages:
        path = debs / Path(item["filename"]).name
        if not path.exists():
            path.write_bytes(assets.fetch(lock["snapshot"] + "/" + item["filename"], item["sha256"]))
        if assets.digest(path.read_bytes()) != item["sha256"]:
            raise RuntimeError("Cached package digest mismatch: " + path.name)
    for name in ("Dockerfile", "entrypoint.sh", "runtime.py"):
        shutil.copyfile(HERE / name, context / name)
    (context / "packages.json").write_text(json.dumps(packages, indent=2) + "\n")
    if source_digest() != before:
        raise RuntimeError("Image sources changed during download; retry with current sources.")
    command = ["docker", "--context", CONTEXT, "buildx", "build", "--builder", CONTEXT, "--load", "--network", "none",
               "--platform", platform, "--build-arg", "BASE=" + lock["base"],
               "--build-arg", "SOURCE_DIGEST=" + before, "--iidfile", str(output / "image.id"), str(context)]
    record = {"sourceDigest": before, "platform": platform, "base": lock["base"], "packages": packages,
              "sources": {str(path.relative_to(ROOT)): assets.digest(path.read_bytes()) for path in SOURCE_FILES},
              "buildCommand": command, "status": "candidate"}
    manifest.write_text(json.dumps(record, indent=2) + "\n")
    subprocess.run(command, check=True, timeout=1800)
    if source_digest() != before:
        raise RuntimeError("Image sources changed during build; image was not selected.")
    record.update(imageId=(output / "image.id").read_text().strip(), status="built")
    manifest.write_text(json.dumps(record, indent=2) + "\n")
    return record


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("linux/arm64", "linux/amd64"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.output.is_absolute():
        parser.error("--output must be an absolute private directory")
    print(json.dumps(build(args.output, args.platform), indent=2))
