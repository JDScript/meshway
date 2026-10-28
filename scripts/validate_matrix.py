#!/usr/bin/env python3
"""Build once, execute the isolated Linux acceptance rows with bounded concurrency."""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import threading
import time

from dev import DevError
from node_lab import ROOT, base_image, docker_target, image_builder, verify_image, sha256
from perf.dataplane import image_for
from validation_support import build_lock, run_id
from verify_runtime import write_json

# The runnable rows in docs/v2/node/validation.md. Lab is not a disposable fixture.
ROWS = {
    "tun": ("tun", ()),
    "packet": ("packet", ()),
    "udp/off": ("runtime", ("--transport", "native-udp", "--dns-backend", "off", "--reported-endpoints")),
    "tcp/off": ("runtime", ("--transport", "native-tcp", "--dns-backend", "off")),
    "vless/off": ("runtime", ("--transport", "vless", "--dns-backend", "off")),
    "trojan/off": ("runtime", ("--transport", "trojan", "--dns-backend", "off")),
    "udp/resolv-conf": ("runtime", ("--transport", "native-udp", "--dns-backend", "resolv-conf")),
    "udp/resolved": ("runtime", ("--transport", "native-udp", "--dns-backend", "resolved")),
    # Ubuntu 22.04 (systemd 249): its resolved cannot serve the resolved
    # backend (the row's resolved check expects the named failure); the
    # documented explicit file proxy runs the whole DNS scenario.
    "jammy/resolv-conf": ("runtime", ("--transport", "native-udp", "--dns-backend", "resolv-conf", "--dns-os", "jammy")),
    "nat/off": ("runtime", ("--transport", "native-udp", "--traversal", "nat", "--dns-backend", "off")),
    "nat+fallback/off": ("runtime", ("--transport", "native-udp", "--traversal", "nat", "--traversal-fallback", "--dns-backend", "off")),
    "nat/resolved": ("runtime", ("--transport", "native-udp", "--traversal", "nat", "--dns-backend", "resolved")),
    "nat/relay": ("runtime", ("--transport", "native-udp", "--traversal", "nat", "--dns-backend", "off", "--netmap-scenario", "relay")),
    "nat/reconnect": ("runtime", ("--transport", "native-udp", "--traversal", "nat", "--dns-backend", "off", "--netmap-scenario", "reconnect")),
    "exit/off": ("runtime", ("--transport", "native-udp", "--traversal", "exit", "--dns-backend", "off")),
    "exit6/off": ("runtime", ("--transport", "native-udp", "--traversal", "exit6", "--dns-backend", "off")),
    "roam/direct": ("runtime", ("--transport", "native-udp", "--traversal", "roam", "--dns-backend", "off")),
    "roam/nat": ("runtime", ("--transport", "native-udp", "--traversal", "roam", "--dns-backend", "off", "--netmap-scenario", "roam-nat")),
    "roam/relay": ("runtime", ("--transport", "native-udp", "--traversal", "roam", "--dns-backend", "off", "--netmap-scenario", "roam-relay")),
    "roam/flap": ("runtime", ("--transport", "native-udp", "--traversal", "roam", "--dns-backend", "off", "--netmap-scenario", "roam-flap")),
    "roam/exit": ("runtime", ("--transport", "native-udp", "--traversal", "roam", "--dns-backend", "off", "--netmap-scenario", "roam-exit")),
    # A→R→B and B's exit E, resolved on every Node, under external pressure
    # (CPU limits, resolved restarts, Control dropped); one bridge.
    "fault/throttle": ("runtime", ("--transport", "native-udp", "--traversal", "fault", "--dns-backend", "off")),
    "netmap/offline": ("runtime", ("--transport", "native-udp", "--dns-backend", "off", "--reported-endpoints", "--netmap-scenario", "offline")),
    "netmap/revoke": ("runtime", ("--transport", "native-tcp", "--dns-backend", "off", "--netmap-scenario", "revoke")),
    # resolved on the runtime image: B's DNS owner fails and recovers past
    # the lease end (SP1).
    "netmap/expiry": ("runtime", ("--transport", "native-udp", "--dns-backend", "resolved", "--netmap-scenario", "expiry")),
    "netmap/restart": ("runtime", ("--transport", "native-udp", "--dns-backend", "off", "--reported-endpoints", "--netmap-scenario", "restart")),
}
# Rows accepted only by explicit selection, with the reason shown in --list.
PENDING_ROWS = {}
DEFAULT_ROWS = tuple(name for name in ROWS if name not in PENDING_ROWS)


def validation_root():
    return Path(os.environ.get("MESHWAY_VALIDATION_ROOT", str(ROOT / ".meshway/validation")))


def positive(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def default_parallel(rows):
    """Rows are not CPU-bound, but every row runs several debug daemons and iperf; on a
    shared host the limit is memory. One quarter of the CPUs, and one row per ~3 GiB of
    currently available memory, keeps a full matrix from starving other tenants."""
    cpus = len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else os.cpu_count() or 1
    limit = max(1, cpus // 4)
    try:
        with open("/proc/meminfo", encoding="ascii") as meminfo:
            available_kib = next(int(line.split()[1]) for line in meminfo if line.startswith("MemAvailable:"))
        limit = max(1, min(limit, available_kib // (3 * 1024 * 1024)))
    except (OSError, StopIteration, ValueError):
        pass
    return max(1, min(len(rows), limit))


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rows", nargs="+", choices=ROWS, default=list(DEFAULT_ROWS))
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--parallel", type=positive, help="concurrent rows; default min(rows, CPUs // 2)")
    parser.add_argument("--short", action="store_true")
    parser.add_argument("--run", default=None, help="unique run ID, also used by remote attach")
    for kind in ("runtime", "packet", "tun"):
        parser.add_argument(f"--prebuilt-{kind}", type=Path, metavar="DIR",
                            help=f"frozen {kind} --build-only output to import instead of building")
    parser.add_argument("--runtime-image-manifest", type=Path)
    parser.add_argument("--dns-image-manifest", type=Path)
    parser.add_argument("--dns-jammy-image-manifest", type=Path)
    parser.add_argument("--traversal-image-manifest", type=Path)
    args = parser.parse_args(argv)
    if args.run and not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,95}", args.run):
        parser.error("invalid run ID")
    args.rows = list(dict.fromkeys(args.rows))
    if args.parallel is None:
        args.parallel = default_parallel(args.rows)
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
    for name in args.rows:
        kind = dns_image(name)
        if kind:
            needed.add(kind)
    if any(name.startswith(("nat", "roam")) for name in args.rows):
        needed.add("traversal")
    images = {}
    for kind in sorted(needed):
        supplied = getattr(args, kind.replace("-", "_") + "_image_manifest")
        if supplied:
            images[kind] = supplied.resolve()
            continue
        builder = image_builder() if kind == "runtime" else None
        lock = ROOT / ("dev/dns-fixture/jammy.lock.json" if kind == "dns-jammy"
                       else f"dev/{kind}-fixture/assets.lock.json")
        digest = builder.source_digest() if builder else sha256(lock)
        output = validation_root() / "os" / (kind + "-" + platform.split("/")[1] + "-" + digest)
        manifest = output / "image-manifest.json"
        with build_lock():
            if manifest.exists():
                data = json.loads(manifest.read_text())
                inspection = subprocess.run(["docker", "image", "inspect", data.get("imageId", "missing")],
                                            capture_output=True, text=True)
                if data.get("status") == "built" and data.get("platform") == platform and inspection.returncode == 0:
                    if builder:
                        # A manifest has one shape: an earlier partial record is replaced below.
                        try:
                            verify_image(data, platform)
                        except DevError:
                            data = None
                    if data is not None:
                        images[kind] = manifest
                        continue
            if builder:
                # Reuse the existing perf lookup of complete, locked native images,
                # including untagged images; it returns the image-build manifest
                # shape (packages read from the image) and the runtime verifier applies.
                data = image_for(platform, output)
                verify_image(data, platform)
                output.mkdir(parents=True, exist_ok=True)
                write_json(manifest, data)
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
    if dns_image(name):
        command += ["--dns-image-manifest", str(images[dns_image(name)])]
    if name.startswith(("nat", "roam")):
        command += ["--traversal-image-manifest", str(images["traversal"])]
    return command


def dns_image(name):
    """The DNS OS image a row's actors run on, if any."""
    if name.startswith("jammy/"):
        return "dns-jammy"
    return "dns" if name.endswith(("/resolved", "/resolv-conf")) else None


def failure_excerpt(report, logfile):
    excerpts = []
    # Actor error.txt is deliberately private: Display errors may contain secrets.
    # Summaries print only its location and the fixture's structured phase fields.
    for path in sorted(report.rglob("error.txt")) if report.exists() else []:
        excerpts.append("private failure: " + str(path.relative_to(report)))
    for path in sorted(report.rglob("report.json")) if report.exists() else []:
        try:
            metadata = json.loads(path.read_text())
        except json.JSONDecodeError:
            excerpts.append("incomplete metadata: " + str(path.relative_to(report)))
            continue
        failed = []
        for group in ("tests", "platformTests"):
            failed.extend(name for name, check in metadata.get(group, {}).items() if check.get("passed") is False)
        failed.extend(name for name in ("bootstrapHttpsTest", "daemonCleanupTest")
                      if metadata.get(name, {}).get("passed") is False)
        if failed:
            excerpts.append(str(path.relative_to(report)) + ": failed checks " + ", ".join(failed))
    for path in sorted(report.rglob("*.log")) if report.exists() else []:
        phases = []
        with path.open("rb") as stream:
            stream.seek(max(0, path.stat().st_size - 32768))
            tail = stream.read().decode(errors="replace")
        for line in tail.splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not isinstance(event, dict) or not isinstance(event.get("fields"), dict):
                continue
            fields = event["fields"]
            stage = {key: fields[key] for key in ("phase", "failure_category") if isinstance(fields.get(key), str)}
            if stage and (not phases or phases[-1] != stage):
                phases.append(stage)
        if phases:
            excerpts.append(str(path.relative_to(report)) + ": " + json.dumps(phases[-3:]))
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


# Docker's default address pools are a shared host resource (31 networks;
# planck's own services hold about six). A row holds its Docker bridges for
# its whole run: exit rows nine plus one for their component checks, roam
# rows four or five, NAT rows three, other runtime rows one. Rows wait for
# their bridges in this budget instead of failing `docker network create`
# (planck: exhausted with four roam rows and the exit row at once, and with
# exit/off beside the NAT and roam rows, matrix 20261017T211436Z-522aba47).
NETWORK_BUDGET = 24
NETWORKS = threading.Condition()
# Roam rows also run one at a time: their timing gates are sensitive to
# one another's network changes.
ROAM_ROWS = threading.Semaphore(1)
NETWORKS_FREE = [NETWORK_BUDGET]


def bridges(name):
    if name.startswith(("exit/", "exit6/")):
        return 10
    if name.startswith("roam/"):
        return 5
    if name.startswith("nat"):
        return 3
    return 0 if name in ("tun", "packet") else 1


# The two longest rows start first, so the matrix's wall clock is not their
# start time plus their length: exit/off (about 13 minutes) and
# fault/throttle (about 12; the others at most about 6, the roam rows one at
# a time). With fault/throttle holding a slot, exit/off in its listed place
# started after 15 minutes and the matrix took 1,541 s (20261022T072926Z-2cbf580a)
# against 1,036 s with both first (20261022T065619Z-34e5d00e). Results keep
# the selected order.
FIRST = ("exit/off", "fault/throttle")


def schedule(rows):
    return sorted(rows, key=lambda name: (FIRST.index(name) if name in FIRST else len(FIRST)))


def execute_row(name, directory, artifacts, images, args):
    need = min(bridges(name), NETWORK_BUDGET)
    with NETWORKS:
        NETWORKS.wait_for(lambda: NETWORKS_FREE[0] >= need)
        NETWORKS_FREE[0] -= need
    try:
        if name.startswith("roam/"):
            with ROAM_ROWS:
                return execute_row_now(name, directory, artifacts, images, args)
        return execute_row_now(name, directory, artifacts, images, args)
    finally:
        with NETWORKS:
            NETWORKS_FREE[0] += need
            NETWORKS.notify_all()


def execute_row_now(name, directory, artifacts, images, args):
    kind = ROWS[name][0]
    row_dir = directory / "rows" / name.replace("/", "-")
    row_dir.mkdir(parents=True)
    report = row_dir / run_id()
    logfile = row_dir / "invocation.log"
    started = time.monotonic()
    result = dict(row=name, passed=False, reportId=report.name, report=str(report.relative_to(directory) / "report.json"),
                   scope="diagnostic-short" if args.short else "acceptance", loadBeforeStart=load())
    if name in PENDING_ROWS:
        result["expectation"] = PENDING_ROWS[name]
    try:
        command = row_command(name, report, artifacts, images, args.short)
        result["command"] = command
        result["exitCode"] = invoke(command, logfile, dict(os.environ, MESHWAY_MATRIX_RUN=directory.name))
        metadata = json.loads((report / "report.json").read_text())
        result["passed"] = result["exitCode"] == 0 and metadata.get("passed") is True
        if name in PENDING_ROWS:
            result["status"] = metadata.get("status", "incomplete")
            result["passed"] = result["passed"] and result["status"] == "passed"
    except Exception as error:
        result["error"] = str(error)
    try:
        result["remainingResources"] = remaining_resources(report, kind)
        if any(result["remainingResources"].values()):
            result["passed"] = False
            result["error"] = "fixture resources remain: " + json.dumps(result["remainingResources"])
    except Exception as error:
        result["cleanupError"] = str(error)
        result["passed"] = False
    result["elapsedSeconds"] = round(time.monotonic() - started, 3)
    if not result["passed"]:
        try:
            result["failureExcerpt"] = failure_excerpt(report, logfile)
        except OSError as error:
            result["failureExcerpt"] = "Private evidence unavailable: " + str(error)
    write_json(row_dir / "row.json", result)
    return result


def summary(report):
    lines = ["# Validation matrix " + report["run"], "", "Scope: `" + report["scope"] + "`",
             f"Elapsed: {report['elapsedSeconds']:.1f}s; parallel: {report['parallel']}; passed: {report['passed']}",
             "", report["loadBeforeStart"]["uptime"], "",
             "| Row | passed | Report ID | Seconds | Start load (1/5/15m) | Failure stage excerpt |",
             "| --- | --- | --- | ---: | --- | --- |"]
    for row in report["rows"]:
        failure = row.get("expectation", "") + " " + row.get("error", "") + " " + row.get("failureExcerpt", "")
        failure = failure.replace("|", "\\|").replace("\n", " ")[:600]
        lines.append(f"| {row['row']} | {row['passed']} | `{row.get('reportId', '—')}` | {row['elapsedSeconds']:.1f} | "
                     + "/".join(str(v) for v in row["loadBeforeStart"]["loadAverage"]) + " | " + failure + " |")
    if report.get("error"):
        lines += ["", "Preparation failure: " + report["error"]]
    return "\n".join(lines) + "\n"


def main(argv=None):
    args = parse_args(argv)
    if args.list:
        for name in ROWS:
            print(name + (" [explicit-only; " + PENDING_ROWS[name] + "]" if name in PENDING_ROWS else ""))
        return 0
    os.umask(0o077)
    directory = validation_root() / "matrix" / (args.run or run_id())
    directory.mkdir(parents=True)
    started = time.monotonic()
    report = dict(version=1, run=directory.name, scope="diagnostic-short" if args.short else "acceptance",
                  parallel=args.parallel, loadBeforeStart=load(), rows=[], passed=False, elapsedSeconds=0,
                  revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                  sourceRoot=str(ROOT),
                  selectedRows=args.rows, builds={}, status="preparing")
    write_json(directory / "matrix.json", report)
    print(f"Matrix run: {directory.name}\n{report['loadBeforeStart']['uptime']}", flush=True)
    try:
        if shutil.disk_usage(ROOT).free < 50 * 1024 ** 3:
            raise RuntimeError("Matrix builds need at least 50 GiB free")
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
            prebuilt = getattr(args, "prebuilt_" + kind)
            if prebuilt:
                artifacts[kind] = prebuilt.resolve()
                report["builds"][kind] = dict(imported=str(artifacts[kind]))
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
            futures = [executor.submit(execute_row, name, directory, artifacts, images, args)
                       for name in schedule(args.rows)]
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
