#!/usr/bin/env python3
"""Capture release L0 profiles without changing production instrumentation."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import uuid

import circuit
from dataplane import image_for
from node_lab import check_elf, docker_target

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--environment", choices=["host", "linux"], required=True)
    parser.add_argument("--rounds", type=int, default=100000)
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args()
    if not 1 <= args.rounds <= 1000000 or not 1 <= args.repeats <= 100:
        parser.error("rounds/repeats outside bounds")
    common = Path(circuit.command("git", "rev-parse", "--git-common-dir")).resolve().parent
    lockpath = common / ".meshway/perf/dataplane/measurement.lock"
    lockpath.parent.mkdir(parents=True, exist_ok=True)
    with lockpath.open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        return measure(args)


def measure(args):
    if shutil.disk_usage(ROOT).free < 50 * 1024**3:
        raise SystemExit("Performance build requires 50 GiB free disk.")
    identity = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8]
    output = ROOT / ".meshway/perf/crypto" / (identity + "-l0-" + args.environment)
    output.mkdir(parents=True)
    cache = ROOT / "target/crypto-l0"
    before = circuit.source_digest()
    build = ["cargo", "build", "--release", "--locked", "-p", "meshway-circuit-perf", "--target-dir", str(cache)]
    binary_source = cache / "release/meshway-circuit-perf"
    environment = {"name": args.environment, "host": platform.platform(), "machine": platform.machine()}
    if args.environment == "linux":
        target, docker_platform, machine = docker_target()
        circuit.command("rustup", "target", "add", target)
        build[1] = "zigbuild"
        build += ["--target", target]
        binary_source = cache / target / "release/meshway-circuit-perf"
        image = image_for(docker_platform, output.parent / "image")
        environment.update(target=target, image=image, cpus=2,
                           kernel=circuit.command("docker", "run", "--rm", "--network", "none", "--cap-drop", "ALL",
                                                  "--entrypoint", "uname", image["imageId"], "-a"))
        probe = subprocess.run(["mise", "exec", "--", "docker", "run", "--rm", "--network", "none",
                                "--cap-drop", "ALL", "--entrypoint", "sh", image["imageId"], "-c", "command -v perf"],
                               cwd=ROOT, capture_output=True, text=True)
        environment["perfAvailable"] = probe.returncode == 0
        environment["profiler"] = "meshway-logging 1/128; no additional production instrumentation"
    else:
        environment["profiler"] = circuit.command("samply", "--version")
    subprocess.run(["mise", "exec", "--", *build], cwd=ROOT, check=True,
                   env=dict(os.environ, CARGO_INCREMENTAL="0", CARGO_TARGET_DIR=str(cache), CARGO_PROFILE_RELEASE_DEBUG="1"))
    if circuit.source_digest() != before:
        raise RuntimeError("source changed during build")
    binary = output / "meshway-circuit-perf"
    shutil.copy2(binary_source, binary)
    binary.chmod(0o500)
    if args.environment == "linux":
        check_elf(binary, machine)
    report = dict(schemaVersion=1, layer="L0", revision=circuit.command("git", "rev-parse", "HEAD"),
                  dirty=bool(circuit.command("git", "status", "--porcelain")), sourceDigest=before,
                  binarySha256=hashlib.sha256(binary.read_bytes()).hexdigest(), platform=environment,
                  rustc=circuit.command("rustc", "-Vv"), options=vars(args),
                  build=dict(profile="release", debug=1, incremental=False, rustflags=os.environ.get("RUSTFLAGS")),
                  results=[], recordings=[])
    for nodes in [2, 3]:
        for repeat in range(args.repeats):
            for rate in ([0, 128] if repeat % 2 == 0 else [128, 0]):
                label = f"n{nodes}-r{repeat}-p{rate}"
                prefix = [str(binary)]
                if args.environment == "linux":
                    prefix = ["docker", "run", "--rm", "--network", "none", "--read-only", "--cap-drop", "ALL",
                              "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "512m",
                              "--env", f"MESHWAY_PROFILE={rate}", "--env", "MESHWAY_PROFILE_DETAIL=1",
                              "--mount", f"type=bind,src={output},dst=/bench,readonly",
                              "--entrypoint", "/bench/meshway-circuit-perf", image["imageId"]]
                invocation = [*prefix, "--nodes", str(nodes), "--packet-bytes", "1280",
                              "--rounds", str(args.rounds), "--warmup", "1000"]
                load_before = os.getloadavg()
                result = subprocess.run(["mise", "exec", "--", *invocation], cwd=ROOT, text=True, capture_output=True,
                                        timeout=180, env=dict(os.environ, MESHWAY_PROFILE=str(rate), MESHWAY_PROFILE_DETAIL="1"))
                (output / (label + ".stdout")).write_text(result.stdout)
                (output / (label + ".stderr")).write_text(result.stderr)
                record = dict(nodes=nodes, packetBytes=1280, repeat=repeat, profileRate=rate, status="failed",
                              hostLoadBefore=load_before, hostLoadAfter=os.getloadavg())
                try:
                    if result.returncode:
                        raise ValueError(f"process exit {result.returncode}")
                    metrics = json.loads(result.stdout)
                    circuit.validate(metrics, argparse.Namespace(rounds=args.rounds, warmup=1000), nodes, 1280, rate)
                    record.update(metrics, status="ok", profile=circuit.profiles(result.stderr) if rate else None)
                except (ValueError, KeyError, TypeError) as error:
                    record["failure"] = str(error)
                report["results"].append(record)
                report["aggregates"] = circuit.aggregate(report["results"])
                circuit.save(output / "results.json", report)
                (output / "summary.md").write_text(circuit.summary(report))
        if args.environment == "host":
            invocation = ["samply", "record", "--save-only", "--unstable-presymbolicate", "--rate", "1000", "--output", str(output / f"n{nodes}.json"),
                          str(binary), "--nodes", str(nodes), "--packet-bytes", "1280", "--rounds", "500000", "--warmup", "1000"]
            result = subprocess.run(["mise", "exec", "--", *invocation], cwd=ROOT, capture_output=True, text=True,
                                    timeout=180, env=dict(os.environ, MESHWAY_PROFILE="0"))
            (output / f"n{nodes}.samply.stdout").write_text(result.stdout)
            (output / f"n{nodes}.samply.stderr").write_text(result.stderr)
            report["recordings"].append(dict(nodes=nodes, command=invocation, exitCode=result.returncode))
    report["sourceUnchangedAfterRun"] = circuit.source_digest() == before
    report["passed"] = (report["sourceUnchangedAfterRun"] and all(r["status"] == "ok" for r in report["results"])
                        and all(r["exitCode"] == 0 for r in report["recordings"]))
    circuit.save(output / "results.json", report)
    print(output, flush=True)
    return int(not report["passed"])


if __name__ == "__main__":
    sys.exit(main())
