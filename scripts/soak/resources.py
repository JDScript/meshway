#!/usr/bin/env python3
"""Hourly: meshwayd idle CPU in the performance.md#idle-cpu method — per-thread
`/proc/<pid>/task/<tid>/schedstat` run time over a 60 s window, the process
total and the reaped children (`nft`/`sh`, cutime+cstime) over the same
window — plus RSS, fd and socket counts and the node netns packet counters
used for the relay/exit budget → samples/<node>/resources/<UTC day>.jsonl.

All nodes are measured in parallel, read-only.

usage: resources.py [--config PATH] [--window SECONDS]
"""
from __future__ import annotations

import sys

import common

# The fixed probe script is gate.RESOURCES (it runs on the node's host); this module parses its output.


def parse(text: str) -> dict:
    """The script's output → one record (percent = one core over the window)."""
    out: dict = {}
    snaps: dict = {}
    current = None
    for line in text.splitlines():
        parts = line.split()
        if not parts:
            continue
        tag = parts[0]
        if tag == "@@error":
            out["error"] = " ".join(parts[1:])
        elif tag == "@@pid":
            pid, ticks, btime, hz, nproc = (int(x) for x in parts[1:6])
            out.update(pid=pid, start=round(btime + ticks / hz, 2), hz=hz, nproc=nproc)
        elif tag in ("@@a", "@@b"):
            current = snaps.setdefault(tag[2:], {"at": float(parts[1]), "children": int(parts[2]),
                                                 "threads": {}, "snmp": {}})
        elif tag == "T" and current is not None and len(parts) >= 3:
            current["threads"][parts[1]] = (int(parts[2]), parts[3] if len(parts) > 3 else "?")
        elif tag == "N" and current is not None:
            proto = parts[1].rstrip(":")
            values = parts[2:]
            if values and not values[0].lstrip("-").isdigit():
                current["snmp"].setdefault("_keys_" + proto, values)
            else:
                keys = current["snmp"].get("_keys_" + proto, [])
                current["snmp"][proto] = dict(zip(keys, (int(v) for v in values)))
        elif tag == "@@mem" and len(parts) >= 3:
            out["rss_kib"], out["hwm_kib"] = int(parts[1]), int(parts[2])
        elif tag == "@@fd" and len(parts) >= 3:
            out["fds"], out["sockets"] = int(parts[1]), int(parts[2])
        elif tag == "@@exe" and len(parts) >= 2:
            out["exe"] = parts[1]
        elif tag == "@@load":
            out["load"] = [float(x) for x in parts[1:4]]
    a, b = snaps.get("a"), snaps.get("b")
    if a and b and "hz" in out:
        window = b["at"] - a["at"]
        out["window_s"] = round(window, 3)
        threads = []
        for tid, (ns_b, name) in b["threads"].items():
            if tid in a["threads"]:
                threads.append({"tid": int(tid), "name": name,
                                "pct": round((ns_b - a["threads"][tid][0]) / 1e9 / window * 100, 3)})
        threads.sort(key=lambda t: -t["pct"])
        out["threads"] = threads
        out["process_pct"] = round(sum(t["pct"] for t in threads), 3)
        out["max_thread_pct"] = threads[0]["pct"] if threads else 0.0
        out["children_pct"] = round((b["children"] - a["children"]) / out["hz"] / window * 100, 3)
        udp_a, udp_b = a["snmp"].get("Udp", {}), b["snmp"].get("Udp", {})
        tcp_a, tcp_b = a["snmp"].get("Tcp", {}), b["snmp"].get("Tcp", {})
        moved = sum(udp_b.get(k, 0) - udp_a.get(k, 0) for k in ("InDatagrams", "OutDatagrams"))
        moved += sum(tcp_b.get(k, 0) - tcp_a.get(k, 0) for k in ("InSegs", "OutSegs"))
        # An upper bound of forwarded packets/s: every forwarded packet is one in + one out, and the netns
        # counters also carry the host's other traffic.
        out["netns_pps"] = round(moved / 2 / window, 2)
    elif "error" not in out:
        out["error"] = "incomplete output"
    return out


def node_resources(config: common.Config, node: str, window: int) -> dict:
    at = common.now()
    result = common.node_call(config, node, "resources", str(window), timeout=window + 60)
    record = {"t": common.iso(at), "node": node, "rc": result.rc}
    record.update(parse(result.out))
    if result.rc != 0 and "error" not in record:
        record["error"] = result.tail(300)
    if "error" in record and result.rc != 0:
        record["error"] += f" (rc {result.rc}: {result.tail(200)})"
    return record


def main(argv: list[str]) -> int:
    config, rest = common.config_from_args(argv)
    window = int(rest[rest.index("--window") + 1]) if "--window" in rest else 60
    records = common.parallel(lambda name: node_resources(config, name, window), sorted(config.nodes))
    ok = True
    for record in records:
        ok = common.append_by_day(config, config.dir("samples", record["node"], "resources"), [record]) and ok
    summary = ", ".join(f"{r['node']} {r.get('max_thread_pct', '?')}/{r.get('process_pct', '?')}%" for r in records)
    common.note(config, "resources", f"{summary}; written {ok}")
    return 0 if ok else 3


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
