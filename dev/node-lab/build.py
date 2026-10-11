#!/usr/bin/env python3
"""Build the locked Ubuntu userspace, independently of Node binary releases."""
import argparse
import hashlib
import importlib.util
import io
import json
import lzma
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

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


PROVENANCE_BUILT = "built here from locked node-lab OS sources"
PROVENANCE_EXISTING = "existing immutable image of locked node-lab OS sources"
PACKAGES = "/usr/share/meshway-lab/packages.json"


def sources():
    return {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in SOURCE_FILES}


def describe(image_id, platform):
    """The manifest of an existing image built from these sources, in the same
    shape as build(); its package list is the one baked into the image."""
    digest = source_digest()
    lock = json.loads((HERE / "assets.lock.json").read_text())
    image = json.loads(subprocess.check_output(["docker", "image", "inspect", image_id], text=True, timeout=45))[0]
    if (image["Os"] + "/" + image["Architecture"] != platform
            or image["Config"].get("Labels", {}).get("io.meshway.node-lab.source") != digest
            or image["Config"].get("Entrypoint") != ["/usr/local/lib/meshway-lab/entrypoint.sh"]):
        raise RuntimeError("Existing image does not come from the current locked OS sources.")
    # A created, never started container: no process, network or mount runs.
    container = subprocess.check_output(["docker", "create", "--network", "none", image["Id"]],
                                        text=True, timeout=45).strip()
    try:
        archive = subprocess.check_output(["docker", "cp", container + ":" + PACKAGES, "-"], timeout=45)
    finally:
        subprocess.run(["docker", "rm", "-f", container], check=True, capture_output=True, timeout=45)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        member = tar.getmember(Path(PACKAGES).name)
        if not member.isfile() or member.size > 1024 * 1024:
            raise RuntimeError("Image package list is not a bounded regular file.")
        packages = json.loads(tar.extractfile(member).read())
    names = {item["name"] for item in packages}
    if (not isinstance(packages, list) or not set(lock["packages"]).issubset(names)
            or any(set(item) != {"name", "version", "architecture", "filename", "sha256"} for item in packages)):
        raise RuntimeError("Image package list does not cover the locked packages.")
    return {"sourceDigest": digest, "platform": platform, "base": lock["base"], "packages": packages,
            "sources": sources(), "buildCommand": None, "provenance": PROVENANCE_EXISTING,
            "imageId": image["Id"], "status": "built"}


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
    for package, executable in lock["requiredTools"].items():
        if package not in lock["packages"] or not executable.startswith(("/usr/bin/", "/usr/sbin/")):
            raise RuntimeError("Required network tools must be explicit locked package inputs.")
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
    (context / "required-tools.json").write_text(json.dumps(lock["requiredTools"], indent=2) + "\n")
    if source_digest() != before:
        raise RuntimeError("Image sources changed during download; retry with current sources.")
    command = ["docker", "--context", CONTEXT, "buildx", "build", "--builder", CONTEXT, "--load", "--network", "none",
               "--platform", platform, "--build-arg", "BASE=" + lock["base"],
               "--build-arg", "SOURCE_DIGEST=" + before, "--iidfile", str(output / "image.id"), str(context)]
    record = {"sourceDigest": before, "platform": platform, "base": lock["base"], "packages": packages,
              "sources": sources(), "buildCommand": command, "provenance": PROVENANCE_BUILT,
              "status": "candidate"}
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
