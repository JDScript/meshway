#!/usr/bin/env python3
"""Source egress capture kernel objects at partition scale, in a disposable netns.

Run as root on the Linux host being measured; nothing outside the namespace
`meshway-r3-<pid>` is touched and the namespace is deleted at the end:

    sudo python3 scripts/perf/capture_sets.py --out <dir> [--sizes 1000,6000,10000,50000]

For each size N it builds N disjoint, non-adjacent IPv4 ranges (deterministic
seed) and measures, with the production command shapes of
`crates/meshway-underlay/src/linux/kernel/egress`:

* nft: one `nft -j -nnn -e -f -` batch creating the capture table with the
  `managed4` interval set and adding all N range elements; one replacement
  batch (`flush set` + N new elements); `nft -j -nnn list table` readback;
  kernel slab growth while the set is installed; peak nft process RSS.
* FIB: the minimal CIDR cover of the same ranges as routes of one private
  table (`ip -batch`, one netlink request with ACK per route, like the
  owner), replacement with `unreachable` routes (pause), dump, flush.

Writes `capture-sets.json` and `capture-sets.md` to --out.
"""
import argparse
import ipaddress
import json
import os
import platform
import random
import statistics
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

NS = f"meshway-r3-{os.getpid()}"
TABLE = "meshway_r3_bench"
ROUNDS = 3


def run(args, data=None, netns=True, check=True):
    """Wall time, result and the child's own peak RSS (KiB, from wait4)."""
    full = (["ip", "netns", "exec", NS] if netns else []) + args
    start = time.perf_counter()
    child = subprocess.Popen(full, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE)
    stdout, stderr = child.communicate(data, timeout=600)
    elapsed = time.perf_counter() - start
    # communicate() already reaped the child; its rusage is gone. Measure the
    # peak in a second, identical run only when asked (see rss()).
    result = subprocess.CompletedProcess(full, child.returncode, stdout, stderr)
    if check and result.returncode:
        raise RuntimeError(f"{args[:3]}: {stderr.decode()[:400]}")
    return elapsed, result, None


def rss(args, data):
    """Peak RSS (KiB) of one run of `args`, measured by /usr/bin/time so the
    child is not forked from this (large) process; None without it."""
    if not Path("/usr/bin/time").exists():
        return None
    result = subprocess.run(["ip", "netns", "exec", NS, "/usr/bin/time", "-f", "%M"] + args,
                            input=data, capture_output=True, timeout=600)
    try:
        return int(result.stderr.decode().strip().splitlines()[-1])
    except (ValueError, IndexError):
        return None


def ranges(count, seed):
    """Disjoint, non-adjacent ranges whose CIDR cover is one or two prefixes
    per range, close to the merged ranges of a country IP set."""
    rng = random.Random(seed)
    span = 1 << (((1 << 32) // count).bit_length() - 1)
    bits = span.bit_length() - 1
    out = []
    for i in range(count):
        size = 1 << rng.randrange(max(0, bits - 8), bits - 2)
        start = i * span + size * rng.randrange(1, span // size - 3)
        out.append((start, start + size * rng.randrange(1, 3) - 1))
    return out


def text(value):
    return str(ipaddress.IPv4Address(value))


def element(start, end):
    return {"range": [text(start), text(end)]} if start != end else text(start)


def create_batch(items):
    return {"nftables": [
        {"create": {"table": {"family": "inet", "name": TABLE}}},
        {"add": {"set": {"family": "inet", "table": TABLE, "name": "managed4",
                         "type": "ipv4_addr", "flags": ["interval"]}}},
        {"add": {"element": {"family": "inet", "table": TABLE, "name": "managed4",
                             "elem": [element(*r) for r in items]}}},
    ]}


def replace_batch(items):
    return {"nftables": [
        {"flush": {"set": {"family": "inet", "table": TABLE, "name": "managed4"}}},
        {"add": {"element": {"family": "inet", "table": TABLE, "name": "managed4",
                             "elem": [element(*r) for r in items]}}},
    ]}


def slab_kib():
    values = {}
    for line in Path("/proc/meminfo").read_text().splitlines():
        key, _, rest = line.partition(":")
        values[key] = int(rest.split()[0])
    return values["Slab"]


def nft_round(items, other):
    """The owner's shapes: echo (`-e`) for the batch creating objects, no echo
    for element replacement, which the owner verifies by readback."""
    echo = ["nft", "-j", "-nnn", "-e", "-f", "-"]
    plain = ["nft", "-j", "-nnn", "-f", "-"]
    run(["nft", "delete", "table", "inet", TABLE], check=False)
    time.sleep(0.2)
    slab_before = slab_kib()
    create = json.dumps(create_batch(items)).encode()
    commit, result, _ = run(plain, create)
    time.sleep(0.5)
    slab_after = slab_kib()
    replace = json.dumps(replace_batch(other)).encode()
    replaced, _, _ = run(plain, replace)
    readback, listed, _ = run(["nft", "-j", "-nnn", "list", "table", "inet", TABLE])
    doc = json.loads(listed.stdout)
    observed = sum(len(i["set"].get("elem", [])) for i in doc["nftables"] if "set" in i)
    peak = rss(plain, replace)
    _, echoed, _ = run(echo, replace, check=False)
    deleted, _, _ = run(["nft", "delete", "table", "inet", TABLE])
    return {
        "commitSeconds": commit, "replaceSeconds": replaced, "readbackSeconds": readback,
        "deleteSeconds": deleted, "batchBytes": len(create), "listBytes": len(listed.stdout),
        "observedElements": observed, "slabKiB": slab_after - slab_before, "nftPeakRssKiB": peak,
        "echoError": echoed.stderr.decode().strip()[:160] if echoed.returncode else None,
        "echoBytes": len(echoed.stdout),
    }


def cover(items):
    nets = []
    for start, end in items:
        nets.extend(ipaddress.summarize_address_range(
            ipaddress.IPv4Address(start), ipaddress.IPv4Address(end)))
    return nets


def fib_round(nets):
    table = "4242"
    run(["ip", "route", "flush", "table", table], check=False)
    add = "".join(f"route add {n} dev r3d0 table {table} proto 252\n" for n in nets).encode()
    added, _, _ = run(["ip", "-batch", "-"], add)
    pause = "".join(f"route replace unreachable {n} table {table} proto 252\n" for n in nets).encode()
    paused, _, _ = run(["ip", "-batch", "-"], pause)
    dumped, listed, _ = run(["ip", "-4", "route", "show", "table", table])
    observed = len(listed.stdout.splitlines())
    flushed, _, _ = run(["ip", "route", "flush", "table", table])
    return {"addSeconds": added, "pauseSeconds": paused, "dumpSeconds": dumped,
            "flushSeconds": flushed, "observedRoutes": observed}


def median(rounds, key):
    values = [r[key] for r in rounds if isinstance(r.get(key), (int, float))]
    if not values:
        return next((r.get(key) for r in rounds if r.get(key) is not None), None)
    return statistics.median(values)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--sizes", default="1000,6000,10000,16384,32768,50000")
    parser.add_argument("--complement-of", type=Path,
                        help="CIDR list (one per line, e.g. geoip:cn): also measure the merged "
                             "IPv4 complement, the managed side of a direct-for-the-set profile")
    args = parser.parse_args()
    if os.geteuid() != 0:
        sys.exit("run as root: the measurement creates and deletes its own netns")
    sizes = [int(s) for s in args.sizes.split(",")]
    args.out.mkdir(parents=True, exist_ok=True)
    subprocess.run(["ip", "netns", "add", NS], check=True)
    try:
        run(["ip", "link", "add", "r3d0", "type", "dummy"])
        run(["ip", "link", "set", "r3d0", "up"])
        results = []
        cases = [(str(n), ranges(n, n), ranges(n, n + 1)) for n in sizes]
        if args.complement_of:
            nets = sorted(ipaddress.ip_network(l.strip()) for l in args.complement_of.read_text().splitlines()
                          if l.strip() and not l.startswith("#") and ":" not in l)
            merged = []
            for net in nets:
                a, b = int(net.network_address), int(net.broadcast_address)
                if merged and a <= merged[-1][1] + 1:
                    merged[-1][1] = max(merged[-1][1], b)
                else:
                    merged.append([a, b])
            complement, at = [], 0
            for a, b in merged:
                if a > at:
                    complement.append((at, a - 1))
                at = b + 1
            if at <= (1 << 32) - 1:
                complement.append((at, (1 << 32) - 1))
            cases.append((f"complement:{args.complement_of.name}", complement, complement[1:]))
        for label, items, other in cases:
            n = len(items)
            nets = cover(items)
            nft_rounds = [nft_round(items, other) for _ in range(ROUNDS)]
            fib_rounds = [fib_round(nets) for _ in range(ROUNDS)]
            row = {"case": label, "ranges": n, "prefixes": len(nets),
                   "nft": {k: median(nft_rounds, k) for k in {k for r in nft_rounds for k in r}},
                   "fib": {k: median(fib_rounds, k) for k in fib_rounds[0]},
                   "rawNft": nft_rounds, "rawFib": fib_rounds}
            results.append(row)
            print(json.dumps({"case": label, "ranges": n, "prefixes": len(nets), "nft": row["nft"],
                              "fib": row["fib"]}), flush=True)
    finally:
        subprocess.run(["ip", "netns", "delete", NS], check=False)
    nft_version = subprocess.run(["nft", "--version"], capture_output=True, text=True).stdout.strip()
    report = {
        "measuredAt": datetime.now(timezone.utc).isoformat(),
        "host": {"kernel": platform.release(), "machine": platform.machine(),
                 "cpus": os.cpu_count(), "nft": nft_version, "node": platform.node()},
        "rounds": ROUNDS, "results": results,
    }
    (args.out / "capture-sets.json").write_text(json.dumps(report, indent=2) + "\n")
    lines = [f"# Capture sets — {report['host']['node']} ({report['host']['kernel']}, {nft_version})", "",
             "| case | ranges | prefixes | nft commit s | nft replace s | nft readback s | list bytes | slab KiB | nft RSS KiB | echo replace | FIB add s | FIB pause s | FIB dump s |",
             "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    def f(value):
        return f"{value:.3f}" if isinstance(value, float) else str(value)
    for r in results:
        a, b = r["nft"], r["fib"]
        lines.append(f"| {r['case']} | {r['ranges']} | {r['prefixes']} | {f(a['commitSeconds'])} | "
                     f"{f(a['replaceSeconds'])} | {f(a['readbackSeconds'])} | {a['listBytes']} | "
                     f"{a['slabKiB']} | {a['nftPeakRssKiB']} | {'fails' if a.get('echoError') else 'ok'} | "
                     f"{f(b['addSeconds'])} | {f(b['pauseSeconds'])} | {f(b['dumpSeconds'])} |")
    (args.out / "capture-sets.md").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
