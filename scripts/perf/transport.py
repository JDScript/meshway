#!/usr/bin/env python3
"""Explicit loopback transport measurements; never invoked by regular checks."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import resource
import shutil
import statistics
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
SOURCE_DIRS = ("crates/meshway-transport", "crates/meshway-transport-stream", "crates/meshway-model",
               "tools/meshway-transport-perf", "scripts/perf")


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def source_digest():
    files = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "mise.toml"]
    for directory in SOURCE_DIRS:
        files.extend(path for path in (ROOT / directory).rglob("*") if path.is_file())
    digest = hashlib.sha256()
    for path in sorted(files):
        if "__pycache__" in path.parts:
            continue
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0")
        digest.update(path.read_bytes() + b"\0")
    return digest.hexdigest()


def save(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")
    temporary.replace(path)


def summary(results):
    rows = ["# Transport performance", "",
            "Same protocol provider (including required TLS/WS); fixed-payload batched echo over loopback.",
            "Datagram mode adds the common framing/ownership layer. Mesh identity authentication,",
            "E2E encryption, routing, dynamic ABI and TUN are not measured by this run.",
            "Table values are mode-wise medians; ratios below are medians of matched repetitions.", "",
            "| Protocol | Payload | Batch | Mode | Goodput Mbps | Messages/s | RTT p50 ms | RTT p99 ms | Child CPU s |",
            "| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |"]
    groups = {}
    for result in results:
        if result["status"] == "ok":
            key = tuple(result[key] for key in ("protocol", "payloadBytes", "batchSize", "mode"))
            groups.setdefault(key, []).append(result)
    for key, samples in sorted(groups.items()):
        med = lambda field: statistics.median(sample[field] for sample in samples)
        rows.append(f"| {key[0]} | {key[1]} | {key[2]} | {key[3]} | {med('goodputMbps'):.3f} | "
                    f"{med('messagesPerSecond'):.0f} | {med('rttP50Ms'):.4f} | {med('rttP99Ms'):.4f} | {med('cpuSeconds'):.4f} |")
    rows.extend(["", "## Paired datagram / raw goodput", ""])
    for protocol, payload, batch in sorted({key[:3] for key in groups}):
        raw = groups.get((protocol, payload, batch, "raw"))
        framed = groups.get((protocol, payload, batch, "datagram"))
        if raw and framed:
            baselines = {r["repeat"]: r for r in raw}
            ratios = [r["goodputMbps"] / baselines[r["repeat"]]["goodputMbps"] for r in framed if r["repeat"] in baselines]
            if ratios:
                rows.append(f"- {protocol}, {payload} bytes, batch {batch}: {statistics.median(ratios):.4%} ({len(ratios)} complete pairs)")
    failures = sum(result["status"] != "ok" for result in results)
    rows.extend(["", f"Failed measurements: {failures}. Allocation/copy counters and peak memory are unmeasured (null).", ""])
    return "\n".join(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--protocol", choices=["tcp", "udp", "vless-tcp", "vless-ws", "trojan", "all"], default="all")
    parser.add_argument("--payload", type=int, nargs="+", default=[64, 1200])
    parser.add_argument("--batch", type=int, nargs="+", default=[1, 32])
    parser.add_argument("--messages", type=int, default=200000)
    parser.add_argument("--warmup", type=int, default=1000)
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args()
    if (not 1 <= args.messages <= 1_000_000 or not 0 <= args.warmup <= 100_000
            or not 1 <= args.repeats <= 100 or any(not 16 <= p <= 65000 for p in args.payload)
            or any(not 1 <= b <= 1024 for b in args.batch)):
        parser.error("messages/warmup/repeats/payload/batch outside measurement bounds")
    common = Path(command("git", "rev-parse", "--git-common-dir"))
    output_root = (ROOT / common).resolve().parent / ".meshway/perf/transport"
    output_root.mkdir(parents=True, exist_ok=True)
    with (output_root / "measurement.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise SystemExit("Another transport measurement is running in this repository.") from None
        return measure(args, output_root)


def measure(args, output_root):
    if shutil.disk_usage(ROOT).free < 50 * 1024 ** 3:
        raise SystemExit("Performance build requires at least 50 GiB free disk space.")
    target = ROOT / "target/transport-perf"
    before = source_digest()
    subprocess.run(["cargo", "build", "--release", "--locked", "-p", "meshway-transport-perf"],
                   cwd=ROOT, check=True, env=dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0"))
    if source_digest() != before:
        raise SystemExit("Measurement sources changed during the build; rerun before measuring.")
    output = output_root / (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8])
    output.mkdir(parents=True)
    metadata = {"schemaVersion": 1, "revision": command("git", "rev-parse", "HEAD"),
                "dirty": bool(command("git", "status", "--porcelain")), "sourceDigest": before,
                "rustc": command("rustc", "-Vv"), "platform": platform.platform(),
                "architecture": platform.machine(), "logicalCpus": os.cpu_count(),
                "runtimeWorkerThreads": 2,
                "scope": "loopback protocol raw versus common datagram adapter",
                "options": vars(args), "results": []}
    # Every sample executes one immutable snapshot, even if another workflow
    # rebuilds the workspace target while this report is running.
    binary = output / "meshway-transport-perf"
    shutil.copy2(target / "release/meshway-transport-perf", binary)
    metadata["binaryFile"] = binary.name
    with binary.open("rb") as stream:
        metadata["binarySha256"] = hashlib.file_digest(stream, "sha256").hexdigest()
    protocols = ["tcp", "udp", "vless-tcp", "vless-ws", "trojan"] if args.protocol == "all" else [args.protocol]
    failed = False
    for protocol in protocols:
        for payload in args.payload:
            for batch in args.batch:
                for repeat in range(args.repeats):
                    for mode in (["raw", "datagram"] if repeat % 2 == 0 else ["datagram", "raw"]):
                        label = f"{protocol}-{payload}-{batch}-{repeat}-{mode}"
                        print(label, flush=True)
                        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
                        with (output / f"{label}.stderr").open("w") as stderr:
                            try:
                                process = subprocess.run([str(binary), "--protocol", protocol, "--mode", mode,
                                                          "--payload-bytes", str(payload), "--batch-size", str(batch),
                                                          "--messages", str(args.messages), "--warmup", str(args.warmup)],
                                                         cwd=ROOT, stdout=subprocess.PIPE, stderr=stderr, text=True, timeout=120)
                            except subprocess.TimeoutExpired:
                                process = None
                        after = resource.getrusage(resource.RUSAGE_CHILDREN)
                        record = {"protocol": protocol, "mode": mode, "payloadBytes": payload, "batchSize": batch,
                                  "repeat": repeat, "status": "failed", "stderr": f"{label}.stderr"}
                        if process is not None:
                            (output / f"{label}.stdout").write_text(process.stdout)
                        if process is not None and process.returncode == 0:
                            try:
                                metrics = json.loads(process.stdout)
                                if (metrics["protocol"] != protocol or metrics["mode"] != mode
                                        or metrics["payloadBytes"] != payload or metrics["batchSize"] != batch
                                        or metrics["receivedMessages"] != args.messages):
                                    raise ValueError("measurement identity or delivery count differs")
                                for field in ("goodputMbps", "messagesPerSecond", "rttP50Ms", "rttP99Ms"):
                                    value = metrics[field]
                                    if isinstance(value, bool) or not isinstance(value, (float, int)) or not math.isfinite(value) or value <= 0:
                                        raise ValueError("invalid measurement")
                                record.update(metrics, status="ok",
                                              cpuSeconds=(after.ru_utime + after.ru_stime) - (usage.ru_utime + usage.ru_stime),
                                              peakMemoryBytes=None, allocations=None, copiedBytes=None)
                            except (KeyError, TypeError, ValueError) as error:
                                record["failure"] = str(error)
                        if record["status"] != "ok":
                            failed = True
                        metadata["results"].append(record)
                        save(output / "results.json", metadata)
                        (output / "summary.md").write_text(summary(metadata["results"]))
    print(output)
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
