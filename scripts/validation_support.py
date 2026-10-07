"""Shared fixture output, immutable artifacts and build-cache ownership."""
import contextlib
import datetime
import fcntl
import functools
import json
import os
from pathlib import Path
import shutil
import sys
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


@functools.lru_cache(maxsize=1)
def rootful_bind_access():
    if sys.platform != "linux" or os.geteuid() == 0:
        return False
    security = json.loads(run("docker", "info", "--format", "{{json .SecurityOptions}}", capture=True))
    return not any("rootless" in option or "userns" in option for option in security)


def bind_access_options():
    # Linux rootful container UID 0 differs from the owner of private host binds.
    # Keep actors root (DNS bus policy and NET_ADMIN), without widening the binds.
    return ["--cap-add", "DAC_OVERRIDE"] if rootful_bind_access() else []


def export_evidence(report, image, platform, kind, identity, paths=None):
    """Return only this run's evidence to its host owner, after writers have joined.

    A ready marker can be exported individually while its router remains alive.
    The helper has no network, production identity, host path beyond this report,
    or privilege beyond CHOWN/DAC_OVERRIDE. Never follows evidence symlinks.
    """
    if not rootful_bind_access():
        return
    if report.is_symlink() or report.stat().st_uid != os.getuid():
        raise RuntimeError("Evidence export requires the current user's report directory")
    targets = ["/evidence"]
    if paths:
        targets = []
        for path in paths:
            relative = path.relative_to(report)
            if ".." in relative.parts:
                raise RuntimeError("Evidence export escaped the owned report")
            targets.append("/evidence/" + relative.as_posix())
    command = ["docker", "create", "--pull", "never", "--platform", platform,
               "--name", f"meshway-{kind}-{identity.lower()}-export-{uuid.uuid4().hex[:8]}",
               "--label", f"io.meshway.{kind}-fixture={identity}",
               "--network", "none", "--read-only", "--cap-drop", "ALL", "--cap-add", "CHOWN",
               "--cap-add", "DAC_OVERRIDE", "--security-opt", "no-new-privileges",
               "--cpus", "1", "--memory", "64m", "--pids-limit", "32", "--entrypoint", "/bin/chown",
               "--mount", f"type=bind,source={report},target=/evidence", image,
               "-hR", f"{os.getuid()}:{os.getgid()}", *targets]
    container = run(*command, capture=True).strip()
    try:
        run("docker", "start", "-a", container, capture=True)
    finally:
        run("docker", "rm", "-f", container, capture=True)
