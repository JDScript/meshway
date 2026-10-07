#!/usr/bin/env python3
"""Explicit L0 protected circuit CPU measurements, outside regular checks."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]


def command(*args):
    return subprocess.check_output(["mise", "exec", "--", *args], cwd=ROOT, text=True).strip()


def source_digest():
    files = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "mise.toml", "mise.lock", "scripts/perf/circuit.py")]
    for name in ("crates", "tools/meshway-static-authority", "tools/meshway-circuit-perf"):
        files.extend(path for path in (ROOT / name).rglob("*") if path.is_file())
    digest = hashlib.sha256()
    for path in sorted(files):
        if "__pycache__" not in path.parts:
            digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def save(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")
    temporary.replace(path)


def quantiles(values):
    ordered = sorted(values)
    def percentile(p):
        rank = (len(ordered) - 1) * p
        lower = math.floor(rank)
        upper = math.ceil(rank)
        return ordered[lower] + (ordered[upper] - ordered[lower]) * (rank - lower)
    return {"samples": len(values), "median": statistics.median(values),
            "p10": percentile(.1), "p90": percentile(.9), "min": ordered[0], "max": ordered[-1]}


def profiles(stderr):
    reports = []
    for line in stderr.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("target") == "meshway_profile":
            reports.append(event["fields"])
    if len(reports) != 2:
        raise ValueError(f"expected pre/post profile pair, got {len(reports)}")
    before, after = reports
    if before["lost"] != 0 or after["lost"] != 0:
        raise ValueError("profile stack/registry lost observations")
    previous = {metric["name"]: metric for metric in json.loads(before["metrics_json"])}
    result = []
    for metric in json.loads(after["metrics_json"]):
        old = previous.get(metric["name"])
        calls = metric["calls_started"] - (old["calls_started"] if old else 0)
        if calls <= 0:
            continue
        times = {}
        for key, value in metric["times"].items():
            # A cumulative maximum cannot be differenced into a window maximum.
            if key == "max_wall_ns":
                continue
            baseline = old["times"][key] if old else None
            times[key] = None if value is None else value - (baseline or 0)
            if times[key] is not None and times[key] < 0:
                raise ValueError("profile cumulative counter decreased")
        result.append({"name": metric["name"], "calls": calls, **times})
    return {"sampleDenominator": after["sample_denominator"], "detail": after["detail"],
            "metrics": result, "lost": after["lost"], "scope": "post-warmup steady-state delta"}


def aggregate(results):
    groups = {}
    for result in results:
        if result["status"] == "ok":
            groups.setdefault((result["nodes"], result["packetBytes"], result["profileRate"]), []).append(result)
    output = []
    for (nodes, size, rate), samples in sorted(groups.items()):
        roles = samples[0]["measured"]["roles"]
        record = {"nodes": nodes, "packetBytes": size, "profileRate": rate,
                  "packetsPerSecond": quantiles([s["packetsPerSecond"] for s in samples]),
                  "harnessCpuNsPerPacket": quantiles([s["harnessCpuNs"] / s["measured"]["deliveredPackets"] for s in samples]),
                  "harnessWallNsPerPacket": quantiles([s["harnessWallNs"] / s["measured"]["deliveredPackets"] for s in samples]),
                  "roles": {}, "profile": [], "frameBytes": samples[0]["measured"]["frameBytes"]}
        for role in roles:
            record["roles"][role] = {
                kind: quantiles([s["measured"]["roles"][role][kind] / s["measured"]["roles"][role]["calls"] for s in samples])
                for kind in ("cpuNs", "wallNs")}
        if rate:
            by_name = {}
            for sample in samples:
                for metric in sample["profile"]["metrics"]:
                    by_name.setdefault(metric["name"], []).append((metric, sample["measured"]["deliveredPackets"]))
            for name, values in by_name.items():
                stage = {"name": name, "sampleCounts": [m["samples"] for m, _ in values],
                         "callsPerPacket": quantiles([m["calls"] / packets for m, packets in values])}
                for kind, count in (("cpu_ns", "cpu_samples"), ("self_cpu_ns", "self_cpu_samples"),
                                    ("wall_ns", "samples"), ("self_wall_ns", "samples")):
                    measured = [(m[kind] / m[count], m["calls"] / packets) for m, packets in values
                                if m[kind] is not None and m[count] > 0]
                    stage[kind + "PerCall"] = quantiles([v for v, _ in measured]) if measured else None
                    stage[kind + "PerPacketEstimate"] = quantiles([v * c for v, c in measured]) if measured else None
                record["profile"].append(stage)
            record["profile"].sort(key=lambda m: (m["self_cpu_nsPerPacketEstimate"] or {"median": -1})["median"], reverse=True)
        output.append(record)
    return output


def summary(metadata):
    rows = ["# L0 protected circuit CPU performance", "",
            "One synchronous thread; real compiler permits, SecureNode handshake, Noise, adjacent MACs,",
            "endpoint flow admission, original output guards and exact byte/path delivery checks. Native UDP carrier",
            "metadata; no socket/TUN/transport adapter or control-plane I/O. One circuit and one established TCP flow.",
            "Each round sends a packet in both directions; 52B = pure ACK, larger packets = contiguous TCP payload.",
            "Role timings include ingress parse/copy, receive/forward and final delivery. Incoming frame release",
            "guard cost is charged to the receiving role. Packet fixture generation/ledger checks are outside role",
            "timers but inside harness CPU/wall throughput. Timer cost is reported, not subtracted.",
            "Quantiles are linear-interpolated over independent process repetitions, not per-packet tail latency.",
            "Profile estimates use exact calls × sampled mean, with nested inclusive/self scopes; they are observer-affected.",
            "", f"Source digest: `{metadata['sourceDigest']}`",
            f"Binary SHA256: `{metadata['binarySha256']}`", f"Platform: {metadata['platform']}",
            f"Parameters: `{json.dumps(metadata['options'], sort_keys=True)}`", "",
            "| Nodes | IP bytes | Profile 1/N | pps median (p10–p90) | A send / receive CPU µs | B send / receive CPU µs | Relay CPU µs/packet | Wire bytes per cut |",
            "| ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |"]
    for group in metadata["aggregates"]:
        roles = group["roles"]
        med = lambda name: roles[name]["cpuNs"]["median"] / 1000
        pps = group["packetsPerSecond"]
        relays = ", ".join(f"{name}: {med(name):.3f}" for name in roles if name.startswith("R")) or "—"
        rows.append(f"| {group['nodes']} | {group['packetBytes']} | {group['profileRate']} | "
                    f"{pps['median']:.0f} ({pps['p10']:.0f}–{pps['p90']:.0f}) | "
                    f"{med('A.send'):.3f} / {med('A.receive'):.3f} | {med('B.send'):.3f} / {med('B.receive'):.3f} | "
                    f"{relays} | {group['frameBytes']['forward']} |")
    for group in metadata["aggregates"]:
        if not group["profileRate"]:
            continue
        rows.extend(["", f"## Profile top: {group['nodes']} nodes, {group['packetBytes']} bytes", "",
                     "Per one-way delivered packet, summed over all path roles. Inclusive columns overlap.", "",
                     "| Stage | Calls/packet | Samples per repeat | Sampled inclusive µs/call | Estimated self CPU µs/packet |",
                     "| --- | ---: | --- | ---: | ---: |"])
        for stage in group["profile"][:15]:
            inclusive = stage["cpu_nsPerCall"]
            own = stage["self_cpu_nsPerPacketEstimate"]
            inc = f"{inclusive['median'] / 1000:.3f}" if inclusive else "unmeasured"
            exc = f"{own['median'] / 1000:.3f}" if own else "unmeasured"
            rows.append(f"| {stage['name']} | {stage['callsPerPacket']['median']:.1f} | {stage['sampleCounts']} | {inc} | {exc} |")
    failures = [r for r in metadata["results"] if r["status"] != "ok"]
    rows.extend(["", f"Failures: {len(failures)}. Failed samples never contribute zeros to aggregates.",
                 "Allocation counts, copied-byte counts and memory peaks are unmeasured (null).", ""])
    return "\n".join(rows)


def validate(metrics, args, nodes, size, rate):
    if (metrics["nodes"] != nodes or metrics["packetBytes"] != size or metrics["rounds"] != args.rounds
            or metrics["warmup"] != args.warmup or metrics["profiling"] != bool(rate)
            or metrics["measured"]["deliveredPackets"] != 2 * args.rounds):
        raise ValueError("measurement identity/delivery mismatch")
    for field in ("packetsPerSecond", "harnessCpuNs", "harnessWallNs"):
        if not isinstance(metrics[field], (int, float)) or not math.isfinite(metrics[field]) or metrics[field] <= 0:
            raise ValueError(f"invalid {field}")
    expected_roles = {"A.send", "A.receive", "B.send", "B.receive"} | {f"R{i}.forward" for i in range(1, nodes - 1)}
    if set(metrics["measured"]["roles"]) != expected_roles:
        raise ValueError("role set mismatch")
    for role, values in metrics["measured"]["roles"].items():
        if values["calls"] != args.rounds * (2 if role.startswith("R") else 1):
            raise ValueError("role count mismatch")
        if values["cpuNs"] <= 0 or values["wallNs"] <= 0:
            raise ValueError("role CPU/wall missing")
    expected = [size + 86 for _ in range(1, nodes)]
    if metrics["measured"]["frameBytes"] != {"forward": expected, "reply": expected}:
        raise ValueError("v5 wire ledger mismatch")


def measure(args, output_root):
    if shutil.disk_usage(ROOT).free < 50 * 1024 ** 3:
        raise SystemExit("Performance build requires at least 50 GiB free disk space.")
    target = ROOT / "target/circuit-perf"
    before = source_digest()
    subprocess.run(["mise", "exec", "--", "cargo", "build", "--release", "--locked", "-p", "meshway-circuit-perf",
                    "--target-dir", str(target)],
                   cwd=ROOT, check=True, env=dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0"))
    if source_digest() != before:
        raise SystemExit("Sources changed during build; rerun.")
    output = output_root / (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8])
    output.mkdir()
    binary = output / "meshway-circuit-perf"
    shutil.copy2(target / "release/meshway-circuit-perf", binary)
    metadata = {"schemaVersion": 1, "layer": "L0", "revision": command("git", "rev-parse", "HEAD"),
                "dirty": bool(command("git", "status", "--porcelain")), "sourceDigest": before,
                "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "binaryFile": binary.name,
                "rustc": command("rustc", "-Vv"), "platform": platform.platform(), "architecture": platform.machine(),
                "logicalCpus": os.cpu_count(), "measurementThreads": 1, "options": vars(args), "results": [],
                "build": {"profile": "release", "cargoIncremental": False,
                          "targetDirectory": str(target.relative_to(ROOT)),
                          "rustflags": os.environ.get("RUSTFLAGS"),
                          "encodedRustflags": os.environ.get("CARGO_ENCODED_RUSTFLAGS")},
                "allocations": None, "copiedBytes": None, "peakMemoryBytes": None}
    rates = [0, args.profile_rate] if args.profile == "both" else [0 if args.profile == "off" else args.profile_rate]
    for nodes in args.nodes:
        for size in args.packet_bytes:
            for repeat in range(args.repeats):
                for rate in (rates if repeat % 2 == 0 else rates[::-1]):
                    label = f"n{nodes}-p{size}-r{repeat}-profile{rate}"
                    print(label, flush=True)
                    record = {"nodes": nodes, "packetBytes": size, "repeat": repeat,
                              "profileRate": rate, "status": "failed", "stderr": label + ".stderr"}
                    try:
                        record["hostLoadBefore"] = os.getloadavg()
                        record["hostUptimeBefore"] = command("uptime")
                        record["hostNprocBefore"] = command("nproc") if sys.platform == "linux" else command("getconf", "_NPROCESSORS_ONLN")
                        process = subprocess.run(["mise", "exec", "--", str(binary), "--nodes", str(nodes),
                                                  "--packet-bytes", str(size), "--rounds", str(args.rounds),
                                                  "--warmup", str(args.warmup)], cwd=ROOT, text=True,
                                                 capture_output=True, timeout=180,
                                                  env=dict(os.environ, MESHWAY_PROFILE=str(rate), MESHWAY_PROFILE_DETAIL="1"))
                        record["hostLoadAfter"] = os.getloadavg()
                        record["hostUptimeAfter"] = command("uptime")
                        record["hostNprocAfter"] = command("nproc") if sys.platform == "linux" else command("getconf", "_NPROCESSORS_ONLN")
                        (output / (label + ".stdout")).write_text(process.stdout)
                        (output / (label + ".stderr")).write_text(process.stderr)
                        if process.returncode:
                            raise ValueError(f"process exit {process.returncode}")
                        metrics = json.loads(process.stdout)
                        validate(metrics, args, nodes, size, rate)
                        record.update(metrics, profile=profiles(process.stderr) if rate else None, status="ok")
                    except subprocess.TimeoutExpired as error:
                        record["failure"] = str(error)
                        for stream in ("stdout", "stderr"):
                            partial = getattr(error, stream) or b""
                            if isinstance(partial, bytes):
                                partial = partial.decode(errors="replace")
                            (output / (label + "." + stream)).write_text(partial)
                    except (ValueError, KeyError, TypeError) as error:
                        record["failure"] = str(error)
                    metadata["results"].append(record)
                    metadata["aggregates"] = aggregate(metadata["results"])
                    save(output / "results.json", metadata)
                    (output / "summary.md").write_text(summary(metadata))
    metadata["sourceUnchangedAfterRun"] = source_digest() == before
    save(output / "results.json", metadata)
    print(output)
    return int(not metadata["sourceUnchangedAfterRun"] or any(r["status"] != "ok" for r in metadata["results"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nodes", type=int, nargs="+", default=[2, 3, 5])
    parser.add_argument("--packet-bytes", type=int, nargs="+", default=[1280, 52])
    parser.add_argument("--rounds", type=int, default=10000)
    parser.add_argument("--warmup", type=int, default=200)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--profile", choices=["off", "on", "both"], default="both")
    parser.add_argument("--profile-rate", type=int, default=128)
    args = parser.parse_args()
    if (not 1 <= args.rounds <= 1_000_000 or not 0 <= args.warmup <= 100_000
            or not 1 <= args.repeats <= 100 or not 1 <= args.profile_rate <= 65536
            or any(not 2 <= n <= 8 for n in args.nodes) or any(not 52 <= p <= 65519 for p in args.packet_bytes)):
        parser.error("nodes/packet-bytes/rounds/warmup/repeats/profile-rate outside bounds")
    # Worktree-local evidence/cache. Never write into the main worktree.
    output_root = ROOT / ".meshway/perf/circuit"
    output_root.mkdir(parents=True, exist_ok=True)
    with (output_root / "measurement.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise SystemExit("Another circuit measurement is running in this worktree.") from None
        return measure(args, output_root)


if __name__ == "__main__":
    sys.exit(main())
