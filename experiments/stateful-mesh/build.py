#!/usr/bin/env python3
"""Isolated, bounded non-root compiler; exact immutable sources and binary receipt."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from performance_lock import performance_lock

TMP = Path("/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode")
COMPILER = "meshway-underlay-compiler:mw-underlay-431-60aab4705c43"
RUNTIME = "meshway-underlay-main-loop:mw-underlay-main-loop-4fad9ff77d71"


def main():
    os.umask(0o077)
    directory = Path(tempfile.mkdtemp(prefix="meshway-stateful-build-", dir=TMP))
    source, build, out = (directory / n for n in ("source", "build", "out"))
    for path in (source, build, out):
        path.mkdir(mode=0o777)
        path.chmod(0o777)
    if len(sys.argv) == 2:
        previous = Path(sys.argv[1]).resolve()
        if previous.parent != TMP or not previous.name.startswith("meshway-stateful-build-"):
            raise ValueError("compiler cache must belong to this fixture under private TMP")
        build = previous / "build"
        if not build.is_dir():
            raise ValueError("compiler cache missing")
    files = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
    files += [str(p.relative_to(ROOT)) for p in (ROOT / "crates/dataplane/src/flow").rglob("*.rs")]
    manifest = {}
    for name in sorted(set(files)):
        if not name or not (name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml") or name.startswith(("crates/", "apps/agent/", "apps/controller/", "apps/apple/core/", "vendor/"))):
            continue
        path = ROOT / name
        if not path.is_file():
            continue
        target = source / name
        target.parent.mkdir(parents=True, exist_ok=True, mode=0o755)
        shutil.copy2(path, target)
        target.chmod(0o644)
        manifest[name] = hashlib.sha256(target.read_bytes()).hexdigest()
    receipt = {"status": "FAIL", "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(), "sources": manifest}
    name = directory.name
    image = f"meshway-managedtest:{name}"
    with performance_lock() as fd:
        def run(args, **kwargs):
            return subprocess.run(args, check=True, pass_fds=(fd,), **kwargs)
        try:
            for kind, ref in (("compiler", COMPILER), ("runtime_base", RUNTIME)):
                receipt[kind] = subprocess.check_output(["docker", "image", "inspect", ref, "--format", "{{.Id}}"], pass_fds=(fd,)).decode().strip()
            script = """set -eu
mkdir -p /build/cargo
export CARGO_HOME=/build/cargo CARGO_TARGET_DIR=/build/target CARGO_BUILD_JOBS=2
export RUSTUP_TOOLCHAIN=1.98.0-aarch64-unknown-linux-gnu
cargo build --locked -p meshway-agent -p meshway-controller --features meshway-agent/managed-connections --bin meshway-agent --bin meshway-controller
cp --sparse=never --reflink=never /build/target/debug/meshway-agent /build/target/debug/meshway-controller /out/
"""
            with (directory / "compiler.log").open("w") as log:
                run(["docker", "run", "--rm", "--name", name, "--user", "1000:1000", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "6g", "--pids-limit", "256", "--read-only", "--tmpfs", "/tmp:rw,nosuid,nodev,size=512m", "--mount", f"type=bind,src={source},dst=/src,readonly", "--mount", f"type=bind,src={build},dst=/build", "--mount", f"type=bind,src={out},dst=/out", "--workdir", "/src", receipt["compiler"], "sh", "-c", script], stdout=log, stderr=subprocess.STDOUT)
            receipt["binaries"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in out.iterdir()}
            base = f'meshway-stateful-base:{receipt["runtime_base"].removeprefix("sha256:")}'
            run(["docker", "tag", receipt["runtime_base"], base])
            shutil.copyfile(ROOT / "experiments/managed-connections/container.py", out / "container.py")
            (out / "Dockerfile").write_text(f'FROM {base}\nRUN apt-get update && apt-get install -y --no-install-recommends iputils-ping libcap2-bin && (if [ -n "$(getcap /usr/bin/ping)" ]; then setcap -r /usr/bin/ping; fi) && test -z "$(getcap /usr/bin/ping)" && rm -rf /var/lib/apt/lists/*\nCOPY meshway-agent meshway-controller /usr/local/bin/\nCOPY container.py /opt/meshway-managed/container.py\nENTRYPOINT ["sleep", "infinity"]\n')
            run(["docker", "build", "-t", image, str(out)])
            receipt["image"] = image
            receipt["image_id"] = subprocess.check_output(["docker", "image", "inspect", image, "--format", "{{.Id}}"], pass_fds=(fd,)).decode().strip()
            receipt["status"] = "PASS"
        finally:
            subprocess.run(["docker", "rm", "-f", name], pass_fds=(fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            (directory / "receipt.json").write_text(json.dumps(receipt, indent=2))
            print(directory, flush=True)
    print(image, flush=True)


if __name__ == "__main__":
    main()
