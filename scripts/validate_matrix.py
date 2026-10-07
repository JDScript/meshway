#!/usr/bin/env python3
"""Build once, execute the isolated Linux acceptance rows with bounded concurrency."""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

from node_lab import ROOT, docker_target, image_builder, verify_image, sha256
from validation_support import build_lock, run_id
from verify_runtime import write_json

# The runnable rows in docs/v2/node/validation.md. Lab is not a disposable fixture.
ROWS = {
    "tun": ("tun", ()),
    "packet": ("packet", ()),
    "udp/off": ("runtime", ("--transport", "native-udp", "--dns-backend", "off")),
    "tcp/off": ("runtime", ("--transport", "native-tcp", "--dns-backend", "off")),
    "udp/resolv-conf": ("runtime", ("--transport", "native-udp", "--dns-backend", "resolv-conf")),
    "udp/resolved": ("runtime", ("--transport", "native-udp", "--dns-backend", "resolved")),
    "nat/off": ("runtime", ("--transport", "native-udp", "--traversal", "nat", "--dns-backend", "off")),
    "nat+fallback/off": ("runtime", ("--transport", "native-udp", "--traversal", "nat", "--traversal-fallback", "--dns-backend", "off")),
    "nat/resolved": ("runtime", ("--transport", "native-udp", "--traversal", "nat", "--dns-backend", "resolved")),
}


def positive(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def default_parallel():
    return max(1, (len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else os.cpu_count() or 1) // 8)


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rows", nargs="+", choices=ROWS, default=list(ROWS))
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--parallel", type=positive, default=default_parallel())
    parser.add_argument("--short", action="store_true")
    parser.add_argument("--run", default=None, help="unique run ID, also used by remote attach")
    parser.add_argument("--runtime-image-manifest", type=Path)
    parser.add_argument("--dns-image-manifest", type=Path)
    parser.add_argument("--traversal-image-manifest", type=Path)
    args = parser.parse_args(argv)
    if args.run and not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,95}", args.run):
        parser.error("invalid run ID")
    args.rows = list(dict.fromkeys(args.rows))
    return args


def load():
    result = subprocess.run(["uptime"], text=True, capture_output=True, check=True)
    return dict(uptime=result.stdout.strip(), loadAverage=list(os.getloadavg()))


def invoke(command, logfile, env=None):
    with logfile.open("w") as log:
        return subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
                              text=True, close_fds=True).returncode


def script(kind):
    return [sys.executable, str(ROOT / "scripts" / f"verify_{kind}.py")]


def prepare_images(args, directory, platform):
    """OS inputs already pin both amd64 and arm64; select the Docker-native arch."""
    needed = {"runtime"} if any(name != "tun" for name in args.rows) else set()
    if any(name.endswith(("/resolved", "/resolv-conf")) for name in args.rows):
        needed.add("dns")
    if any(name.startswith("nat") for name in args.rows):
        needed.add("traversal")
    images = {}
    for kind in sorted(needed):
        supplied = getattr(args, kind + "_image_manifest")
        if supplied:
            images[kind] = supplied.resolve()
            continue
        builder = image_builder() if kind == "runtime" else None
        lock = ROOT / f"dev/{kind}-fixture/assets.lock.json"
        digest = builder.source_digest() if builder else sha256(lock)
        output = ROOT / ".meshway/validation/os" / (kind + "-" + platform.split("/")[1] + "-" + digest)
        manifest = output / "image-manifest.json"
        with build_lock():
            if manifest.exists():
                data = json.loads(manifest.read_text())
                inspection = subprocess.run(["docker", "image", "inspect", data.get("imageId", "missing")],
                                            capture_output=True, text=True)
                if data.get("status") == "built" and data.get("platform") == platform and inspection.returncode == 0:
                    images[kind] = manifest
                    continue
            if builder:
                verify_image(builder.build(output, platform), platform)
            else:
                command = [sys.executable, str(ROOT / "dev/dns-fixture/build.py"), "--platform", platform,
                           "--lock-file", str(lock), "--output", str(output)]
                if manifest.exists():
                    command.append("--resume")
                if invoke(command, directory / (kind + "-image.log")):
                    raise RuntimeError(kind + " image build failed; inspect " + kind + "-image.log")
        images[kind] = manifest
    return images


def row_command(name, report, artifacts, images, short):
    kind, options = ROWS[name]
    command = script(kind) + ["--report-dir", str(report), "--artifacts", str(artifacts[kind]), *options]
    if kind == "packet":
        command += ["--runtime-artifacts", str(artifacts["runtime"])]
    if kind != "tun":
        command += ["--runtime-image-manifest", str(images["runtime"])]
        if short:
            command.append("--short")
    for image_kind, needed in (("dns", name.endswith(("/resolved", "/resolv-conf"))),
                               ("traversal", name.startswith("nat"))):
        if needed:
            command += ["--" + image_kind + "-image-manifest", str(images[image_kind])]
    return command


def failure_excerpt(report, logfile):
    excerpts = []
    # Actor error files contain the precise first failure, unlike a generic attach exit.
    for path in sorted(report.rglob("error.txt")) if report.exists() else []:
        excerpts.append(str(path.relative_to(report)) + ": " + path.read_text(errors="replace")[-1600:])
    if logfile.exists():
        excerpts.append("invocation: " + "\n".join(logfile.read_text(errors="replace").splitlines()[-12:]))
    return "\n".join(excerpts)[-7000:]


def remaining_resources(report, kind):
    identities = [(kind, report.name)]
    if kind == "packet" and (report / "daemon").exists():
        identities.extend(("runtime", child.name) for child in (report / "daemon").iterdir() if child.is_dir())
    resources = {"containers": [], "networks": []}
    for fixture, identity in identities:
        label = f"io.meshway.{fixture}-fixture={identity}"
        for key, command in (("containers", ["docker", "ps", "-aq"]), ("networks", ["docker", "network", "ls", "-q"])):
            result = subprocess.run([*command, "--filter", "label=" + label], text=True, capture_output=True, check=True)
            resources[key].extend(result.stdout.split())
    return resources


def execute_row(name, directory, artifacts, images, args):
    kind = ROWS[name][0]
    row_dir = directory / "rows" / name.replace("/", "-")
    row_dir.mkdir(parents=True)
    report = row_dir / run_id()
    logfile = row_dir / "invocation.log"
    started = time.monotonic()
    result = dict(row=name, passed=False, reportId=report.name, report=str(report.relative_to(directory) / "report.json"),
                  scope="diagnostic-short" if args.short else "acceptance", loadBeforeStart=load())
    try:
        command = row_command(name, report, artifacts, images, args.short)
        result["command"] = command
        result["exitCode"] = invoke(command, logfile, dict(os.environ, MESHWAY_MATRIX_RUN=directory.name))
        metadata = json.loads((report / "report.json").read_text())
        result["passed"] = result["exitCode"] == 0 and metadata.get("passed") is True
    except Exception as error:
        result["error"] = str(error)
    try:
        result["remainingResources"] = remaining_resources(report, kind)
        if any(result["remainingResources"].values()):
            result["passed"] = False
    except Exception as error:
        result["cleanupError"] = str(error)
        result["passed"] = False
    result["elapsedSeconds"] = round(time.monotonic() - started, 3)
    if not result["passed"]:
        result["failureExcerpt"] = failure_excerpt(report, logfile)
    write_json(row_dir / "row.json", result)
    return result


def summary(report):
    lines = ["# Validation matrix " + report["run"], "", "Scope: `" + report["scope"] + "`",
             f"Elapsed: {report['elapsedSeconds']:.1f}s; parallel: {report['parallel']}; passed: {report['passed']}",
             "", report["loadBeforeStart"]["uptime"], "",
             "| Row | passed | Report ID | Seconds | Start load (1/5/15m) | Failure stage excerpt |",
             "| --- | --- | --- | ---: | --- | --- |"]
    for row in report["rows"]:
        failure = row.get("error", "") + " " + row.get("failureExcerpt", "")
        failure = failure.replace("|", "\\|").replace("\n", " ")[:600]
        lines.append(f"| {row['row']} | {row['passed']} | `{row.get('reportId', '—')}` | {row['elapsedSeconds']:.1f} | "
                     + "/".join(str(v) for v in row["loadBeforeStart"]["loadAverage"]) + " | " + failure + " |")
    if report.get("error"):
        lines += ["", "Preparation failure: " + report["error"]]
    return "\n".join(lines) + "\n"


def main(argv=None):
    args = parse_args(argv)
    if args.list:
        for name in args.rows:
            print(name)
        return 0
    os.umask(0o077)
    directory = ROOT / ".meshway/validation/matrix" / (args.run or run_id())
    directory.mkdir(parents=True)
    started = time.monotonic()
    report = dict(version=1, run=directory.name, scope="diagnostic-short" if args.short else "acceptance",
                  parallel=args.parallel, loadBeforeStart=load(), rows=[], passed=False, elapsedSeconds=0,
                  revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                  selectedRows=args.rows, builds={}, status="preparing")
    write_json(directory / "matrix.json", report)
    print(f"Matrix run: {directory.name}\n{report['loadBeforeStart']['uptime']}", flush=True)
    try:
        _, platform, _ = docker_target()
        report["platform"] = platform
        images = prepare_images(args, directory, platform)
        kinds = {ROWS[name][0] for name in args.rows}
        if "packet" in kinds:
            kinds.add("runtime")
        artifacts = {}
        for kind in ("runtime", "packet", "tun"):
            if kind not in kinds:
                continue
            destination = directory / "builds" / kind / run_id()
            build_started = time.monotonic()
            code = invoke(script(kind) + ["--build-only", "--report-dir", str(destination)], directory / (kind + "-build.log"))
            report["builds"][kind] = dict(exitCode=code, elapsedSeconds=time.monotonic() - build_started)
            if code:
                raise RuntimeError(kind + " build failed; inspect " + kind + "-build.log")
            artifacts[kind] = destination if kind == "tun" else destination / "bin"
        report["status"] = "running"
        write_json(directory / "matrix.json", report)
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.parallel) as executor:
            futures = [executor.submit(execute_row, name, directory, artifacts, images, args) for name in args.rows]
            for future in concurrent.futures.as_completed(futures):
                row = future.result()
                report["rows"].append(row)
                report["elapsedSeconds"] = time.monotonic() - started
                write_json(directory / "matrix.json", report)
                print(f"{row['row']}: passed={row['passed']} {row['elapsedSeconds']:.1f}s", flush=True)
        report["passed"] = all(row["passed"] for row in report["rows"])
    except Exception as error:
        report["error"] = str(error)
    finally:
        report["rows"].sort(key=lambda row: args.rows.index(row["row"]))
        report["elapsedSeconds"] = time.monotonic() - started
        report["status"] = "complete"
        write_json(directory / "matrix.json", report)
        (directory / "summary.md").write_text(summary(report))
        print(summary(report), flush=True)
        print(directory, flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
