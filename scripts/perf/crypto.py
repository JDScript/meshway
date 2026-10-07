#!/usr/bin/env python3
"""Explicit release primitive measurements on host and isolated Linux (no network)."""
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


def command(*args):
    return circuit.command(*args)


def digest():
    paths = [ROOT / p for p in ("Cargo.toml", "Cargo.lock", "mise.toml", "mise.lock",
                               "scripts/perf/crypto.py", "scripts/perf/circuit.py",
                               "scripts/perf/dataplane.py", "scripts/node_lab.py")]
    for directory in ("tools/meshway-crypto-bench", "crates/meshway-logging"):
        paths.extend(p for p in (ROOT / directory).rglob("*") if p.is_file())
    sha = hashlib.sha256()
    for path in sorted(paths):
        sha.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    return sha.hexdigest()


def aggregate(samples):
    grouped = {}
    for sample in samples:
        for field in ("cpuNs", "wallNs", "iterations"):
            if not isinstance(sample[field], int) or sample[field] <= 0:
                raise ValueError("invalid primitive timing/count")
        key = sample["operation"], sample["bytes"]
        grouped.setdefault(key, []).append(sample)
    return [{"operation": op, "bytes": size,
             "cpuNsPerOp": circuit.quantiles([s["cpuNs"] / s["iterations"] for s in rows]),
             "wallNsPerOp": circuit.quantiles([s["wallNs"] / s["iterations"] for s in rows])}
            for (op, size), rows in sorted(grouped.items())]


def summary(report):
    lines = ["# Cryptographic primitive CPU measurements", "",
             f"Revision: `{report['revision']}`; source digest: `{report['sourceDigest']}`.",
             "One synchronous thread; release default target CPU/features, no forced backend cfg.",
             "CPU = thread CPU over fixed iteration batch; setup, validation and 1000 warmup operations excluded.",
             "All AEAD rows include new key object per operation. inplace seal reuses a mutable allocation;",
             "open.inplace includes restoring ciphertext with copy_from_slice. alloc rows include clone/allocation/drop.",
             "AAD tag = empty plaintext, input as AAD, 96-bit nonce = zero32 || sequence LE64.",
             "AEAD seal/tag uses increasing nonce within each timed stream. Repeated open verifies a fixed valid ciphertext.",
             "Only public fixed benchmark keys/data; cross-provider ciphertext/tag and plaintext equality checked before timing.",
             "MAC input lengths below are primitive lengths, not v5 IP lengths: production MAC also authenticates a prefix and AEAD tag.",
             "No timer/copy baseline is subtracted. Quantiles cover repeated batches, not operation tail latency.",
             "x86_64: unmeasured (no native host). aws-lc-rs: unmeasured.", ""]
    for environment in report["environments"]:
        lines += [f"## {environment['name']}", "", f"Status: {environment['status']}",
                  f"Binary: `{environment.get('binarySha256')}`", "",
                  "| Operation | Bytes | CPU µs/op median | p10–p90 | Wall µs/op |",
                  "| --- | ---: | ---: | ---: | ---: |"]
        for row in environment.get("aggregates", []):
            cpu = row["cpuNsPerOp"]
            lines.append(f"| {row['operation']} | {row['bytes']} | {cpu['median']/1000:.4f} | "
                         f"{cpu['p10']/1000:.4f}–{cpu['p90']/1000:.4f} | {row['wallNsPerOp']['median']/1000:.4f} |")
        if environment.get("failure"):
            lines += ["", environment["failure"]]
    return "\n".join(lines) + "\n"


def measure(args, output_root):
    if shutil.disk_usage(ROOT).free < 50 * 1024**3:
        raise SystemExit("Performance build requires 50 GiB free disk.")
    identity = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8]
    output = output_root / identity
    output.mkdir()
    cache = ROOT / "target/crypto-perf"
    before = digest()
    report = dict(schemaVersion=1, revision=command("git", "rev-parse", "HEAD"),
                  dirty=bool(command("git", "status", "--porcelain")), sourceDigest=before,
                  options=vars(args), rustc=command("rustc", "-Vv"),
                  host=platform.platform(), hostMachine=platform.machine(),
                  rustflags=os.environ.get("RUSTFLAGS"), encodedRustflags=os.environ.get("CARGO_ENCODED_RUSTFLAGS"),
                  environments=[], x86_64=None, awsLc=None)
    for name in args.environment:
        env = dict(name=name, status="failed")
        report["environments"].append(env)
        try:
            build = ["cargo", "build", "--release", "--locked", "-p", "meshway-crypto-bench", "--example", "measure",
                     "--target-dir", str(cache)]
            prefix = []
            executable = cache / "release/examples/measure"
            if name == "linux":
                target, docker_platform, machine = docker_target()
                command("rustup", "target", "add", target)
                build[1] = "zigbuild"
                build += ["--target", target]
                executable = cache / target / "release/examples/measure"
                image = image_for(docker_platform, output_root / "image")
                env.update(target=target, image=image, cpus=args.cpus,
                           zig=command("zig", "version"), cargoZigbuild=command("cargo-zigbuild", "--version"))
            subprocess.run(["mise", "exec", "--", *build], cwd=ROOT, check=True,
                           env=dict(os.environ, CARGO_INCREMENTAL="0", CARGO_TARGET_DIR=str(cache)))
            if digest() != before:
                raise RuntimeError("source changed during build")
            binary = output / ("measure-" + name)
            shutil.copy2(executable, binary)
            binary.chmod(0o500)
            env["binarySha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
            if name == "linux":
                check_elf(binary, machine)
                prefix = ["docker", "run", "--rm", "--name", "meshway-crypto-" + identity.lower(),
                          "--network", "none", "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                          "--cpus", str(args.cpus), "--memory", "512m", "--platform", docker_platform,
                          "--mount", f"type=bind,src={output},dst=/bench,readonly", "--entrypoint", "/bench/" + binary.name,
                          image["imageId"]]
                env["kernel"] = command("docker", "run", "--rm", "--network", "none", "--cap-drop", "ALL",
                                        "--entrypoint", "uname", image["imageId"], "-a")
            else:
                prefix = [str(binary)]
            invocation = [*prefix, "--iterations", str(args.iterations), "--repeats", str(args.repeats)]
            env["command"] = invocation
            process = subprocess.run(["mise", "exec", "--", *invocation], cwd=ROOT, text=True, capture_output=True,
                                     timeout=1800, env=dict(os.environ, MESHWAY_PROFILE="0"))
            (output / (name + ".stdout")).write_text(process.stdout)
            (output / (name + ".stderr")).write_text(process.stderr)
            if process.returncode:
                raise RuntimeError(f"benchmark exit {process.returncode}: {process.stderr}")
            env["samples"] = json.loads(process.stdout)
            env["aggregates"] = aggregate(env["samples"])
            env["status"] = "ok"
        except (RuntimeError, ValueError, KeyError, subprocess.SubprocessError) as error:
            env["failure"] = str(error)
        circuit.save(output / "results.json", report)
        (output / "summary.md").write_text(summary(report))
    report["sourceUnchangedAfterRun"] = digest() == before
    report["passed"] = report["sourceUnchangedAfterRun"] and all(e["status"] == "ok" for e in report["environments"])
    circuit.save(output / "results.json", report)
    print(output, flush=True)
    return int(not report["passed"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--environment", nargs="+", choices=["host", "linux"], default=["host", "linux"])
    parser.add_argument("--iterations", type=int, default=100000)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--cpus", type=float, default=2)
    args = parser.parse_args()
    if not 100 <= args.iterations <= 10000000 or not 1 <= args.repeats <= 100 or not 0 < args.cpus <= 256:
        parser.error("iterations/repeats/cpus outside bounds")
    output_root = ROOT / ".meshway/perf/crypto"
    output_root.mkdir(parents=True, exist_ok=True)
    common = Path(command("git", "rev-parse", "--git-common-dir")).resolve().parent
    # Share the Linux dataplane measurement lock across worktrees.
    lockpath = common / ".meshway/perf/dataplane/measurement.lock"
    lockpath.parent.mkdir(parents=True, exist_ok=True)
    with lockpath.open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        return measure(args, output_root)


if __name__ == "__main__":
    sys.exit(main())
