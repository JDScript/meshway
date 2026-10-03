#!/usr/bin/env python3
"""Package source-verified native binaries without compiling Rust in BuildKit."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import uuid

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "apps/agent/tests/underlay_fixture"))
from performance_lock import LOCK_FD_ENV, performance_lock, validate_descriptor
from source_manifest import inventory, verify
from docker_lifecycle import Creation

TEMP = Path("/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode")


def architecture(value):
    value = {"aarch64": "arm64", "x86_64": "amd64"}.get(value, value)
    if value not in ("arm64", "amd64"):
        raise ValueError("unsupported native Docker architecture")
    return value


def check_image(image, engine):
    if (not re.fullmatch(r"sha256:[0-9a-f]{64}", image["Id"])
            or image.get("Os") != "linux"
            or architecture(image.get("Architecture")) != architecture(engine)
            or image.get("Config", {}).get("Volumes")):
        raise ValueError("expected a native Linux binary image without state volumes")


def build(args):
    owner = uuid.uuid4().hex
    source = args.source_root.resolve()
    expected = inventory(source)
    with performance_lock() as descriptor, tempfile.TemporaryDirectory(
            prefix="meshway-runtime-", dir=TEMP) as temporary:
        directory = Path(temporary)
        os.chmod(directory, 0o700)

        def docker(*command, timeout=120, creation=None):
            validate_descriptor(descriptor)
            def invoke():
                return subprocess.run(["docker", *command], cwd=ROOT, text=True,
                    capture_output=True, timeout=timeout, pass_fds=(descriptor,),
                    env={**os.environ, "DOCKER_CONTEXT": "default", LOCK_FD_ENV: str(descriptor)})
            result = creation.perform(invoke) if creation else invoke()
            if result.returncode:
                raise RuntimeError(result.stderr[-4000:])
            return result.stdout.strip()

        images = json.loads(docker("image", "inspect", args.binary_image))
        if len(images) != 1:
            raise ValueError("binary image reference is ambiguous")
        binary_image = images[0]
        check_image(binary_image, docker("info", "--format", "{{.Architecture}}"))
        if not re.search(r"^Driver:\s+docker\s*$", docker("buildx", "inspect", "default"), re.M):
            raise ValueError("runtime packaging requires the ordinary Docker builder")
        source_id = binary_image["Id"]
        tags = {}
        for line in docker("image", "ls", "--all", "--no-trunc", "--format", "{{json .}}").splitlines():
            entry = json.loads(line)
            reference = entry["Repository"] + ":" + entry["Tag"]
            tags.setdefault(reference, set()).add(entry["ID"])
        if args.tag in tags:
            raise ValueError("output image tag already exists; choose a unique tag")
        extraction = Creation("container", "meshway-runtime-extract-" + owner,
            "io.meshway.runtime-build", owner, image=source_id, network="none", require_stopped=True)
        try:
            docker("create", "--name", extraction.name, "--network", "none", "--cap-drop", "ALL",
                "--label", "io.meshway.runtime-build=" + owner, "--entrypoint", "/bin/true", source_id,
                creation=extraction)
            manifest = directory / "source-manifest.json"
            docker("cp", extraction.name + ":/source-manifest.json", str(manifest))
            if not manifest.is_file() or manifest.is_symlink():
                raise ValueError("source manifest must be a regular file")
            verify(json.loads(manifest.read_text()), expected)
        finally:
            handlers = {sig: signal.signal(sig, signal.SIG_IGN) for sig in (signal.SIGINT, signal.SIGTERM)}
            try:
                extraction.cleanup(docker)
            finally:
                for sig, handler in handlers.items():
                    signal.signal(sig, handler)
        # Retain an immutable, source-ID-derived name. BuildKit resolves a named
        # local image rather than interpreting a bare image ID as a registry tag.
        binary_reference = "meshway-runtime-binary:" + source_id.removeprefix("sha256:")
        if binary_reference in tags and tags[binary_reference] != {source_id}:
            raise ValueError("binary retention tag belongs to another image")
        if binary_reference not in tags:
            docker("image", "tag", source_id, binary_reference)
        recipe = "Dockerfile.dev" if args.developer else "Dockerfile"
        files = [recipe, "runtime-bootstrap", "container.py", "meshway-dev-launcher"]
        hashes = {}
        for name in files:
            path = HERE / name
            if path.is_symlink():
                raise ValueError("runtime inputs must not be symlinks")
            hashes[name] = hashlib.sha256(path.read_bytes()).hexdigest()
            shutil.copy2(path, directory / name)
        runtime = {"format_version": 1, "binary_image": source_id, "binary_reference": binary_reference,
            "source_manifest_sha256": hashlib.sha256((directory / "source-manifest.json").read_bytes()).hexdigest(),
            "recipe": recipe, "runtime_files": hashes}
        (directory / "runtime-manifest.json").write_text(json.dumps(runtime, sort_keys=True))
        # Only ordinary runtime layers are built here. The binary image must be
        # produced separately by the bounded, non-root Linux compilation runner.
        docker("build", "--builder", "default", "--build-arg", "MESHWAY_BINARY_IMAGE=" + binary_reference,
            "--label", "io.meshway.runtime-build=" + owner, "--tag", args.tag,
            "--file", str(directory / recipe), str(directory), timeout=600)
        built = json.loads(docker("image", "inspect", args.tag))[0]
        check_image(built, binary_image["Architecture"])
        if built["Config"]["Labels"].get("io.meshway.runtime-build") != owner:
            raise RuntimeError("output image ownership differs")
        print(json.dumps({"image": built["Id"], "tag": args.tag, "binary_image": source_id,
            "source_root": str(source), "source_files": len(expected["files"]), "runtime": recipe}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary-image", required=True, help="existing source-manifest-bearing native image")
    parser.add_argument("--tag", required=True, help="unique meshway-managedtest:TAG output")
    parser.add_argument("--developer", action="store_true", help="package the writable Ubuntu developer variant")
    parser.add_argument("--source-root", type=Path, default=ROOT,
        help="exact source tree to verify; defaults to this checkout")
    args = parser.parse_args()
    if not re.fullmatch(r"meshway-managedtest:[a-zA-Z0-9_.-]+", args.tag):
        parser.error("output must use the dedicated meshway-managedtest repository")
    build(args)


def interrupted(_signal, _frame):
    raise KeyboardInterrupt("runtime packaging interrupted")


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    main()
