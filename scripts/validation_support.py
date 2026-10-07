"""Shared fixture output, immutable artifacts and build-cache ownership."""
import contextlib
import datetime
import fcntl
import json
import os
from pathlib import Path
import shutil
import uuid

from node_lab import ROOT, check_elf, run, sha256


def run_id():
    return datetime.datetime.now(datetime.UTC).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]


def fixture_options(parser):
    parser.add_argument("--report-dir", type=Path, help="new private report directory")
    parser.add_argument("--build-only", action="store_true")
    parser.add_argument("--artifacts", type=Path)


def report_directory(kind, supplied=None):
    common = Path(run("git", "rev-parse", "--git-common-dir", capture=True).strip())
    repository = (ROOT / common).resolve().parent
    report = supplied.resolve() if supplied else repository / ".meshway/validation" / kind / run_id()
    report.mkdir(parents=True, mode=0o700)
    return report


@contextlib.contextmanager
def build_lock():
    # All three fixtures share this Cargo target, even from standalone entrypoints.
    path = ROOT / "target/node-kernel-tests/build.lock"
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as lock:
        os.set_inheritable(lock.fileno(), False)
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def save_artifact(directory, binary, source, target, platform):
    manifest = dict(sourceDigest=source, sourceRoot=str(ROOT.resolve()), target=target,
                    platform=platform, sha256={binary.name: sha256(binary)})
    (directory / "source-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def import_artifact(directory, destination, source, target, platform, machine):
    manifest = json.loads((directory / "source-manifest.json").read_text())
    binary = directory / destination.name
    if (manifest.get("sourceDigest") != source or manifest.get("sourceRoot") != str(ROOT.resolve())
            or manifest.get("target") != target or manifest.get("platform") != platform
            or manifest.get("sha256") != {binary.name: sha256(binary)}):
        raise RuntimeError("Prebuilt fixture does not match source/target/hash")
    check_elf(binary, machine)
    shutil.copyfile(binary, destination)
    destination.chmod(0o500)
