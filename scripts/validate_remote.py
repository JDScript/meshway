#!/usr/bin/env python3
"""Detached remote matrix execution, reconnectable waiting and private evidence retrieval."""
import argparse
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import time

from node_lab import ROOT
from validation_support import run_id
from validation_progress import display as progress_display

DEFAULT_HOST = "zj2631@ai4ce-planck.dyn.poly.edu"

# Runs as a session leader. Each run owns a fresh detached worktree, so concurrent
# runs never replace each other's sources; only git operations on the shared clone
# are serialized. Fixture binaries are built in one fixed-path build worktree (Cargo
# fingerprints include the workspace path, so per-run paths would rebuild every
# workspace crate), cached per revision and imported by digest into each run.
WORKER = r'''
import contextlib, fcntl, json, os, pathlib, subprocess, sys, time, traceback
p = json.loads(sys.argv[1])
root = pathlib.Path(p["path"])
validation = root / ".meshway/validation"
job = validation / "jobs" / p["run"]
matrix = validation / "matrix" / p["run"]
workspace = root / ".worktree" / ("validation-" + p["run"])
os.umask(0o077)
(job / "pid").write_text(str(os.getpid()) + "\n")
result = {"run": p["run"], "revision": p["revision"], "exitCode": 1, "status": "running"}
def save():
    temporary = job / "result.tmp"
    temporary.write_text(json.dumps(result, indent=2) + "\n")
    temporary.replace(job / "result.json")
def call(command, cwd=root):
    return subprocess.check_output(command, cwd=cwd, text=True, stderr=subprocess.STDOUT).strip()
@contextlib.contextmanager
def flock(name):
    with (validation / name).open("a") as lock:
        os.set_inheritable(lock.fileno(), False)
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield
def git_lock():
    # Short: fetch / worktree add / worktree remove only, never the matrix itself.
    return flock("git.lock")
KINDS = ("runtime", "packet", "tun")
def prebuilt(kind):
    directory = validation / "builds" / p["revision"] / kind
    return directory if kind == "tun" else directory / "bin"
def build_fixtures(env):
    # Serialized across runs: the build worktree is checked out to one revision at a time.
    with flock("build-worktree.lock"):
        build_root = root / ".worktree/validation-build"
        with git_lock():
            if not build_root.exists():
                call(["git", "worktree", "add", "--detach", str(build_root), p["revision"]])
            else:
                call(["git", "-C", str(build_root), "checkout", "--detach", p["revision"]])
        if call(["git", "status", "--porcelain"], cwd=build_root):
            raise RuntimeError("build worktree must be clean")
        for kind in KINDS:
            if (prebuilt(kind) / "source-manifest.json").exists():
                continue
            destination = validation / "builds" / p["revision"] / kind
            destination.parent.mkdir(parents=True, exist_ok=True)
            build_env = dict(env, MISE_TRUSTED_CONFIG_PATHS=str(build_root))
            log = job / (kind + "-build.log")
            with log.open("w") as output:
                # The test:<kind>:linux tasks carry the zig / cargo-zigbuild tool set.
                code = subprocess.call(["mise", "run", "test:" + kind + ":linux", "--build-only", "--report-dir", str(destination)],
                                       cwd=build_root, env=build_env, stdout=output, stderr=subprocess.STDOUT)
            if code:
                raise RuntimeError(kind + " build failed; inspect " + log.name)
save()
created = False
try:
    result["uptime"] = call(["uptime"])
    print(result["uptime"], flush=True)
    with git_lock():
        call(["git", "fetch", "origin"])
        workspace.parent.mkdir(parents=True, exist_ok=True)
        call(["git", "worktree", "add", "--detach", str(workspace), p["revision"]])
        created = True
    result["workspace"] = str(workspace)
    save()
    if call(["git", "status", "--porcelain"], cwd=workspace):
        raise RuntimeError("validation worktree must be clean")
    temporary = validation / "tmp" / p["run"]
    temporary.mkdir(parents=True)
    # The remote is a shared host: cap Cargo's parallelism so a cold build cannot push it into OOM.
    env = dict(os.environ, CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS=os.environ.get("MESHWAY_REMOTE_BUILD_JOBS", "8"),
               MISE_TRUSTED_CONFIG_PATHS=str(workspace),
               MESHWAY_VALIDATION_ROOT=str(validation), MESHWAY_VALIDATION_TARGET=str(validation / "target"),
               TMPDIR=str(temporary))
    env["PATH"] = str(pathlib.Path.home() / ".local/bin") + ":" + env["PATH"]
    build_fixtures(env)
    result["builds"] = {kind: str(prebuilt(kind)) for kind in KINDS}
    save()
    imports = [option for kind in KINDS for option in ("--prebuilt-" + kind, str(prebuilt(kind)))]
    result["exitCode"] = subprocess.call(["mise", "run", "validate:matrix", "--run", p["run"], *imports, *p["arguments"]],
                                         cwd=workspace, env=env)
    data = json.loads((matrix / "matrix.json").read_text())
    # Independently query Docker after the matrix ends, including nested packet runtime.
    remaining = {"containers": [], "networks": []}
    for row in data["rows"]:
        kind = "runtime" if "/" in row["row"] else row["row"]
        report = matrix / row["report"]
        identities = [(kind, row["reportId"])]
        daemon = report.parent / "daemon"
        if daemon.exists():
            identities.extend(("runtime", child.name) for child in daemon.iterdir() if child.is_dir())
        for kind, identity in identities:
            label = "label=io.meshway." + kind + "-fixture=" + identity
            for key, command in (("containers", ["docker", "ps", "-aq"]), ("networks", ["docker", "network", "ls", "-q"])):
                remaining[key].extend(call([*command, "--filter", label]).split())
    result["remainingResources"] = remaining
    if data.get("status") != "complete" or not data.get("passed") or any(remaining.values()):
        result["exitCode"] = 1
except BaseException as error:
    result["error"] = str(error)
    traceback.print_exc()
finally:
    # Evidence lives under .meshway/validation; the worktree only held sources.
    if created:
        try:
            with git_lock():
                call(["git", "worktree", "remove", "--force", str(workspace)])
            result["workspaceRemoved"] = True
        except BaseException as error:
            result["workspaceRemoved"] = False
            result["cleanupError"] = str(error)
            result["exitCode"] = 1
            traceback.print_exc()
    result["status"] = "complete"
    matrix.mkdir(parents=True, exist_ok=True)
    (matrix / "remote.json").write_text(json.dumps(result, indent=2) + "\n")
    if not (matrix / "summary.md").exists():
        (matrix / "summary.md").write_text("# Remote validation failed\n\n" + result.get("error", "matrix did not produce a summary") + "\n")
    save()
'''

LAUNCH = r'''
import json, os, pathlib, subprocess, sys
p = json.loads(sys.argv[1])
os.umask(0o077)
job = pathlib.Path(p["path"]) / ".meshway/validation/jobs" / p["run"]
job.mkdir(parents=True)
(job / "tmp").mkdir()
(job / "request.json").write_text(json.dumps(p, indent=2) + "\n")
with (job / "worker.log").open("w") as log:
    process = subprocess.Popen([sys.executable, "-c", sys.argv[2], json.dumps(p)],
                               stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                               start_new_session=True, close_fds=True,
                               env=dict(os.environ, TMPDIR=str(job / "tmp")))
(job / "launcher-pid").write_text(str(process.pid) + "\n")
print(p["run"])
'''


def ssh(host, command, timeout=60):
    return subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15", host,
                           shlex.join(command)], text=True, capture_output=True, timeout=timeout, check=True).stdout


def pushed_revision(ref):
    revision = subprocess.check_output(["git", "rev-parse", "--verify", ref + "^{commit}"], cwd=ROOT, text=True).strip()
    advertised = subprocess.check_output(["git", "ls-remote", "planck"], cwd=ROOT, text=True)
    for line in advertised.splitlines():
        tip = line.split()[0]
        if tip == revision:
            return revision
        # A previously pushed commit can also be an ancestor of an advertised tip.
        if subprocess.run(["git", "merge-base", "--is-ancestor", revision, tip], cwd=ROOT,
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
            return revision
    raise RuntimeError("Ref is not reachable from git ls-remote planck; push it to planck first")


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default=DEFAULT_HOST)
    parser.add_argument("--path", default="/mnt/data/Meshway/src")
    parser.add_argument("--ref", default="HEAD")
    parser.add_argument("--attach", help="resume waiting for an existing run")
    parser.add_argument("--timeout", type=float, default=14400, help="local wait seconds; remote execution continues")
    args, forwarded = parser.parse_known_args(argv)
    if not re.fullmatch(r"[A-Za-z0-9_.@-]+", args.host) or args.host.startswith("-"):
        parser.error("invalid SSH host")
    if not args.path.startswith("/") or "\n" in args.path:
        parser.error("--path must be an absolute remote directory")
    if args.attach and not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,95}", args.attach):
        parser.error("invalid attach run ID")
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    if forwarded[:1] == ["--"]:
        forwarded = forwarded[1:]
    if any(item == "--run" or item.startswith("--run=") for item in forwarded):
        parser.error("remote owns --run; use --attach to resume")
    if args.attach and forwarded:
        parser.error("--attach uses the recorded matrix arguments")
    return args, forwarded


def main(argv=None):
    args, forwarded = parse_args(argv)
    os.umask(0o077)
    identity = args.attach or run_id()
    job = args.path + "/.meshway/validation/jobs/" + identity
    destination = ROOT / ".meshway/validation/remote" / (args.host + "-" + identity)
    if destination.exists():
        raise RuntimeError("Local evidence already exists; refusing to overwrite " + str(destination))
    print(f"Remote run: {identity}\nResume: mise run validate:remote --host {args.host} --path {shlex.quote(args.path)} --attach {identity}", flush=True)
    if not args.attach:
        revision = pushed_revision(args.ref)
        payload = dict(path=args.path, run=identity, revision=revision, arguments=forwarded)
        # Launch returns immediately; all long-lived descriptors belong to the remote session.
        ssh(args.host, ["python3", "-c", LAUNCH, json.dumps(payload), WORKER])
    started = time.monotonic()
    last_progress = None
    progress_reader = Path(__file__).with_name("validation_progress.py").read_text()
    while True:
        try:
            snapshot = json.loads(ssh(args.host, ["python3", "-c", progress_reader,
                                                  args.path + "/.meshway/validation", identity]))
            result = snapshot.get("result") or {}
            progress = snapshot.get("progress")
            if progress is not None:
                elapsed = progress.get("elapsedSeconds")
                marker = (progress.get("status"), json.dumps(progress.get("rows")),
                          None if elapsed is None else elapsed // 60)
                if marker != last_progress:
                    print(progress_display(progress), flush=True)
                    last_progress = marker
            if result.get("status") == "complete":
                break
        except (subprocess.SubprocessError, json.JSONDecodeError) as error:
            print(f"Waiting for remote result ({type(error).__name__}); reconnecting", flush=True)
        if time.monotonic() - started > args.timeout:
            print("Local wait timed out. Remote job continues; use the --attach command above.", file=sys.stderr)
            return 2
        time.sleep(15)
    print("Remote execution finished; retrieving private evidence...", flush=True)
    destination.mkdir(parents=True)
    # Evidence and hashes are retained; multi-GiB executable copies stay on the remote host.
    command = ["rsync", "-a", "--exclude=bin/", "--exclude=linux-tun-tests", "--exclude=builds/",
               "-e", "ssh -o BatchMode=yes -o ConnectTimeout=15",
               args.host + ":" + shlex.quote(args.path + "/.meshway/validation/matrix/" + identity + "/"), str(destination) + "/"]
    try:
        subprocess.run(command, check=True)
        subprocess.run(["rsync", "-a", "-e", "ssh -o BatchMode=yes -o ConnectTimeout=15",
                        args.host + ":" + shlex.quote(job + "/"), str(destination / "remote-job") + "/"], check=True)
    except subprocess.SubprocessError:
        # A partial download is still evidence. A subsequent attach gets a fresh, non-overwriting directory.
        partial = destination.with_name(destination.name + "-partial-" + run_id())
        destination.rename(partial)
        raise
    print((destination / "summary.md").read_text())
    print("Local evidence:", destination)
    return result["exitCode"]


if __name__ == "__main__":
    sys.exit(main())
