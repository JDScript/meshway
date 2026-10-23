#!/usr/bin/env python3
"""Daily report, final gate verdict and the daily probes of the field soak.

usage:
  report.py [--date YYYY-MM-DD] [--no-collect]   daily report (default: yesterday UTC) → reports/<day>.md/.json
  report.py --final [--no-collect]                gate verdict over the whole soak → reports/final.md/.json
  report.py probe                                 `meshway ping` ×10 per edge and one rate-limited iperf3
                                                  → samples/probes/<UTC day>.jsonl (cron 23:30Z)
  (all accept --config PATH)

Before reporting, one log collection pass runs under the logs lock so the
day's tail is stored. Events are labelled planned when a faults.jsonl window
(start … end + grace) of a fault that affects the node covers them.
"""
from __future__ import annotations

import json
import statistics
import sys
import threading
import time
from datetime import timedelta

import common

WITHDRAW = {
    "Required DNS component failed; suspending packet plane",
    "Required packet plane component failed; suspending packet plane",
    "Netmap withdrawn; terminating the packet plane",
    "Netmap lease ended; terminating the packet plane",
}
REINSTALL = {
    "Required DNS component recovered; netmap reinstalled",
    "Required packet plane component recovered; netmap reinstalled",
    "Netmap installed again after its withdrawal",
    "Kernel ownership recovered; reinstalling the netmap",
}
SYNC_FAILED = "Node sync failed; retrying"
REACHABILITY = {"Network", "ServerBusy", "network", "serverBusy"}
PRIORITY = {0: "ERROR", 1: "ERROR", 2: "ERROR", 3: "ERROR", 4: "WARN", 5: "INFO", 6: "INFO", 7: "DEBUG"}
SAMPLE_INTERVAL_S = 300


# ---------------------------------------------------------------- faults

def fault_windows(config: common.Config) -> list[dict]:
    path = config.dir("faults.jsonl")
    events = common.read_jsonl(path) if path.exists() else []
    windows: dict = {}
    for event in events:
        fid = event["id"]
        if event["event"] == "start":
            windows[fid] = {"id": fid, "start": common.parse_time(event["t"]), "end": None, "affects": event["affects"],
                            "grace": timedelta(minutes=event.get("grace_min", 15)), "ok": None,
                            "planned_at": event.get("planned_at"), "action": event.get("action"), "dry": event.get("dry")}
        elif event["event"] == "end" and fid in windows:
            windows[fid].update(end=common.parse_time(event["t"]), ok=event["ok"], error=event.get("error"),
                                detail=event.get("detail"), interrupted=event.get("interrupted"))
        elif event["event"] == "skipped":
            windows[fid] = {"id": fid, "skipped": event.get("reason"), "start": None, "end": None, "affects": [],
                            "grace": timedelta(0), "planned_at": event.get("planned_at")}
    return [w for w in windows.values() if not w.get("dry")]


def planned(windows: list[dict], node: str, start, end=None) -> str | None:
    """The fault whose window (start … end + grace; open → now) overlaps [start, end] for this node."""
    end = end or start
    for window in windows:
        if not window["start"] or not (node in window["affects"] or ("*" in window["affects"] and node != "server")):
            continue
        close = (window["end"] or common.now()) + window["grace"]
        if window["start"] <= end and start <= close:
            return window["id"]
    return None


# ---------------------------------------------------------------- status samples

def status_analysis(records: list[dict], baseline: dict | None, windows: list[dict], node: str) -> dict:
    out = {"samples": len(records), "unreachable": 0, "phases": {}, "sync": {}, "sync_errors": {},
           "blocked": [], "starts": [], "lease_renewals": 0, "min_lease_left_h": None, "serials": [],
           "revisions": [], "sync_error_samples": []}
    previous = baseline
    episode = None
    last_issued = (((baseline or {}).get("status") or {}).get("netmap") or {}).get("issuedAt")
    last_issued = common.parse_time(last_issued) if last_issued else None
    last_serial = (((baseline or {}).get("status") or {}).get("applied") or {}).get("serial")
    last_revision = (((baseline or {}).get("status") or {}).get("applied") or {}).get("revision")
    for record in records:
        at = common.parse_time(record["t"])
        status = record.get("status")
        if not status:
            out["unreachable"] += 1
            continue
        phase = status.get("phase")
        out["phases"][phase] = out["phases"].get(phase, 0) + 1
        out["sync"][status.get("sync")] = out["sync"].get(status.get("sync"), 0) + 1
        if status.get("syncError"):
            out["sync_errors"][status["syncError"]] = out["sync_errors"].get(status["syncError"], 0) + 1
            out["sync_error_samples"].append({"t": record["t"], "error": status["syncError"],
                                              "planned": planned(windows, node, at)})
        identity = process_identity(record)
        before = process_identity(previous) if previous else None
        if identity and before and not same_process(identity, before):
            since = common.parse_time(previous["t"])
            out["starts"].append({"t": record["t"], "after": previous["t"], "pid": identity[0],
                                  "previous_pid": before[0], "planned": planned(windows, node, since, at)})
        if identity:
            previous = record
        if phase == "blocked":
            if episode is None:
                episode = {"start": record["t"], "end": record["t"], "samples": 0, "causes": [],
                           "planned": planned(windows, node, at)}
                out["blocked"].append(episode)
            episode["end"] = record["t"]
            episode["samples"] += 1
            cause = [status.get("lastError"), status.get("lastErrorDetail")]
            if cause not in episode["causes"]:
                episode["causes"].append(cause)
        else:
            episode = None
        netmap = status.get("netmap") or {}
        issued = common.parse_time(netmap["issuedAt"]) if netmap.get("issuedAt") else None
        if issued and issued != last_issued:
            if last_issued is not None:
                out["lease_renewals"] += 1
            last_issued = issued
        if netmap.get("expiresAt"):
            left = (common.parse_time(netmap["expiresAt"]) - at).total_seconds() / 3600
            if out["min_lease_left_h"] is None or left < out["min_lease_left_h"]:
                out["min_lease_left_h"] = round(left, 2)
        applied = status.get("applied") or {}
        if applied.get("serial") is not None and applied["serial"] != last_serial:
            if last_serial is not None:
                out["serials"].append({"t": record["t"], "from": last_serial, "to": applied["serial"]})
            last_serial = applied["serial"]
        if applied.get("revision") is not None and applied["revision"] != last_revision:
            if last_revision is not None:
                out["revisions"].append({"t": record["t"], "from": last_revision, "to": applied["revision"]})
            last_revision = applied["revision"]
    for item in out["blocked"]:
        item["has_cause"] = all(cause[0] for cause in item["causes"])
    out["last"] = previous
    return out


def same_process(a, b) -> bool:
    return a[0] == b[0] and (a[1] is None or b[1] is None or a[1] == b[1])


def process_identity(record: dict | None):
    if not record:
        return None
    proc = record.get("proc") or {}
    status = record.get("status") or {}
    pid = status.get("processId") or proc.get("pid")
    if not pid:
        return None
    # The start time only belongs to the daemon when the probed PID is the one that answered Status.
    return (pid, proc.get("start") if proc.get("pid") == pid else None)


def server_analysis(records: list[dict], baseline: dict | None, windows: list[dict]) -> dict:
    out = {"samples": len(records), "health_ok": 0, "health_failed": [], "starts": []}
    started = ((baseline or {}).get("container") or {}).get("startedAt")
    for record in records:
        health = record.get("health")
        if health is not None:
            if health.get("ok"):
                out["health_ok"] += 1
            else:
                at = common.parse_time(record["t"])
                out["health_failed"].append({"t": record["t"], "http": health.get("http"), "error": health.get("error"),
                                             "planned": planned(windows, "server", at)})
        container = record.get("container") or {}
        if container.get("startedAt"):
            if started and container["startedAt"] != started:
                moment = common.parse_time(container["startedAt"])
                out["starts"].append({"t": container["startedAt"], "planned": planned(windows, "server", moment)})
            started = container["startedAt"]
    return out


# ---------------------------------------------------------------- logs

def parse_log(line: dict) -> tuple[str, str, dict]:
    """(level, message, fields) of a stored log line (JSON tracing or plain text)."""
    text = line.get("msg", "")
    if text.startswith("{"):
        try:
            obj = json.loads(text)
            fields = obj.get("fields") or {}
            return obj.get("level", "?"), str(fields.get("message", "")), fields
        except json.JSONDecodeError:
            pass
    level = PRIORITY.get(line.get("prio"), "TEXT") if "prio" in line else "TEXT"
    return level, text.strip(), {}


def category(level: str, message: str, fields: dict) -> str:
    key = f"{level} {message[:120]}"
    for name in ("error", "problem", "cause"):
        value = fields.get(name)
        if isinstance(value, str) and len(value) <= 40:
            return f"{key} [{value}]"
    return key


def log_analysis(lines: list[dict], windows: list[dict], node: str) -> dict:
    out = {"lines": 0, "levels": {}, "categories": {}, "withdrawals": [], "reinstalls": [], "sync_failures": [],
           "blocked": [], "systemd": []}
    seen = set()
    for line in lines:
        key = (line.get("t"), line.get("pid"), line.get("msg"))
        if key in seen:  # a batch repeated after an interrupted collection
            continue
        seen.add(key)
        out["lines"] += 1
        level, message, fields = parse_log(line)
        out["levels"][level] = out["levels"].get(level, 0) + 1
        at = common.parse_time(line["t"])
        tag = planned(windows, node, at)
        if level in ("WARN", "ERROR"):
            name = category(level, message, fields)
            out["categories"][name] = out["categories"].get(name, 0) + 1
        brief = {k: v for k, v in fields.items() if k != "message" and isinstance(v, (str, int, float, bool))}
        if message in WITHDRAW:
            out["withdrawals"].append({"t": line["t"], "message": message, "fields": brief, "planned": tag})
        elif message in REINSTALL:
            out["reinstalls"].append({"t": line["t"], "message": message, "fields": brief, "planned": tag})
        elif message == SYNC_FAILED:
            error = str(fields.get("error", "?"))
            out["sync_failures"].append({"t": line["t"], "error": error, "planned": tag,
                                         "reachability": error in REACHABILITY})
        if fields.get("phase") == "Blocked":
            out["blocked"].append({"t": line["t"], "message": message, "fields": brief, "planned": tag})
        if line.get("sys") and (message.startswith("Started ") or "Main process exited" in message
                                or "Scheduled restart job" in message):
            out["systemd"].append({"t": line["t"], "message": message[:160], "planned": tag})
    return out


def read_logs(config: common.Config, node: str, day: str) -> list[dict]:
    return common.read_day(config.dir("logs", node), day)


# ---------------------------------------------------------------- resources

def resource_analysis(records: list[dict], windows: list[dict], node: str, spec: dict, thresholds: dict) -> dict:
    thread_limit = thresholds.get("thread_pct", 5.0)
    children_limit = thresholds.get("children_pct", 5.0)
    fixed = spec.get("fixed_pct", thresholds.get("fixed_pct", 8.0))
    per_packet_us = thresholds.get("per_packet_us", 29.0)
    relay = bool(spec.get("relay_or_exit"))
    out = {"samples": len(records), "missing": 0, "violations": [], "excluded": 0, "values": []}
    for record in records:
        if "max_thread_pct" not in record:
            out["missing"] += 1
            continue
        at = common.parse_time(record["t"])
        tag = planned(windows, node, at, at + timedelta(seconds=record.get("window_s", 60)))
        value = {k: record.get(k) for k in ("t", "max_thread_pct", "process_pct", "children_pct", "rss_kib", "fds",
                                            "sockets", "netns_pps", "pid", "start", "exe")}
        value["planned"] = tag
        out["values"].append(value)
        if tag:
            out["excluded"] += 1
            continue
        if record["max_thread_pct"] >= thread_limit:
            busiest = record["threads"][0] if record.get("threads") else {}
            out["violations"].append({"t": record["t"], "gate": f"thread < {thread_limit} %",
                                      "value": record["max_thread_pct"], "thread": busiest.get("name")})
        if relay:
            budget = fixed + (record.get("netns_pps") or 0) * per_packet_us * 1e-6 * 100
            if record["process_pct"] > budget:
                out["violations"].append({"t": record["t"], "gate": f"process ≤ {fixed} % + pps × {per_packet_us} µs",
                                          "value": record["process_pct"], "budget": round(budget, 2)})
            if record["children_pct"] > children_limit:
                out["violations"].append({"t": record["t"], "gate": f"nft children ≤ {children_limit} %",
                                          "value": record["children_pct"]})
    for metric in ("max_thread_pct", "process_pct", "children_pct", "rss_kib", "fds", "sockets"):
        series = [v[metric] for v in out["values"] if v.get(metric) is not None]
        out[metric] = {"median": round(statistics.median(series), 3), "max": max(series)} if series else None
    return out


def slope_per_day(points: list[tuple[float, float]]) -> float:
    """Least-squares slope; x in hours → value per day."""
    n = len(points)
    mean_x = sum(p[0] for p in points) / n
    mean_y = sum(p[1] for p in points) / n
    var = sum((p[0] - mean_x) ** 2 for p in points)
    if var == 0:
        return 0.0
    return sum((p[0] - mean_x) * (p[1] - mean_y) for p in points) / var * 24


def trend(values: list[dict], metric: str, thresholds: dict, horizon_days: float = 7) -> list[dict]:
    """Per process segment (same pid/start) with ≥ min_hours of samples: sustained growth verdict.

    Growth is sustained when BOTH the fitted slope extrapolated over the horizon exceeds the limit AND the
    median of the segment's last 24 h exceeds the median of its first 24 h by the same limit's quarter.
    Limits (thresholds.json overrides): RSS max(8 MiB, 10 % of the first-24 h median); fds/sockets 8.
    """
    segments: dict = {}
    for value in values:
        if value.get(metric) is None:
            continue
        segments.setdefault((value.get("pid"), value.get("start")), []).append(value)
    out = []
    min_hours = thresholds.get("trend_min_hours", 24)
    for (pid, _), items in segments.items():
        items.sort(key=lambda v: v["t"])
        first_t = common.parse_time(items[0]["t"])
        points = [((common.parse_time(v["t"]) - first_t).total_seconds() / 3600, float(v[metric])) for v in items]
        span = points[-1][0]
        if span < min_hours or len(points) < 6:
            out.append({"pid": pid, "metric": metric, "hours": round(span, 1), "verdict": "too short"})
            continue
        head = statistics.median([y for x, y in points if x <= 24])
        tail = statistics.median([y for x, y in points if x >= span - 24])
        if metric == "rss_kib":
            limit = max(thresholds.get("rss_floor_kib", 8192), thresholds.get("rss_fraction", 0.10) * head)
        else:
            limit = thresholds.get(f"{metric}_limit", 8)
        slope = slope_per_day(points)
        projected = slope * horizon_days
        growing = projected > limit and tail - head > limit / 4
        out.append({"pid": pid, "metric": metric, "hours": round(span, 1), "slope_per_day": round(slope, 3),
                    "projected": round(projected, 2), "limit": round(limit, 2), "first_24h": head, "last_24h": tail,
                    "verdict": "growing" if growing else "flat"})
    return out


# ---------------------------------------------------------------- probes

def ping_edge(config: common.Config, edge: list[str]) -> dict:
    source, target = edge
    destination = config.nodes[target].get("mesh_name", target) if target in config.nodes else target
    result = common.run_node(config, source, ["meshway", "--json", "ping", "-c", "10", destination], 120)
    record = {"t": common.iso(common.now()), "kind": "ping", "src": source, "dst": target, "rc": result.rc}
    for line in result.out.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "completed":
            record.update({k: event.get(k) for k in ("sent", "success", "timeout", "failed")})
        elif event.get("type") == "failed":
            record["error"] = event.get("code")
        elif event.get("type") == "decision":
            decision = event.get("decision") or {}
            record["path"] = len(decision.get("path") or [])
            record["action"] = (decision.get("action") or {}).get("type")
    if "success" not in record and "error" not in record:
        record["error"] = result.tail(300)
    return record


def throughput(config: common.Config) -> dict | None:
    spec = config.data.get("throughput")
    if not spec:
        return None
    port, seconds, mbit = spec.get("port", 5201), spec.get("seconds", 10), spec.get("mbit", 50)
    record = {"t": common.iso(common.now()), "kind": "throughput", "src": spec["client"], "dst": spec["server"],
              "mbit_limit": mbit, "seconds": seconds}
    server_result = {}

    def serve():
        server_result["r"] = common.run_node(config, spec["server"], ["timeout", str(seconds + 50), "iperf3", "-s",
                                                                       "-1", "-p", str(port)], seconds + 60)
    thread = threading.Thread(target=serve)
    thread.start()
    time.sleep(3)
    client = common.run_node(config, spec["client"], ["iperf3", "-c", spec["target"], "-p", str(port), "-t",
                                                      str(seconds), "-b", f"{mbit}M", "-J"], seconds + 40)
    thread.join(seconds + 70)
    try:
        report = json.loads(client.out)
        if report.get("error"):
            record["error"] = report["error"]
        else:
            end = report.get("end", {})
            record["received_mbit"] = round(end["sum_received"]["bits_per_second"] / 1e6, 2)
            record["retransmits"] = end.get("sum_sent", {}).get("retransmits")
    except (json.JSONDecodeError, KeyError, TypeError):
        record["error"] = client.tail(300) or (server_result.get("r").tail(200) if server_result.get("r") else "")
    return record


def probe(config: common.Config) -> int:
    edges = config.data.get("edges", [])
    records = common.parallel(lambda edge: ping_edge(config, edge), edges)
    rate = throughput(config)
    if rate:
        records.append(rate)
    ok = common.append_by_day(config, config.dir("samples", "probes"), records)
    common.note(config, "probe", ", ".join(f"{r['src']}>{r['dst']} {r.get('success', r.get('received_mbit', 'x'))}"
                                           for r in records))
    return 0 if ok else 3


# ---------------------------------------------------------------- analysis and gates

def window_of(config: common.Config, day: str):
    start, end = common.day_start(day), common.day_start(day) + timedelta(days=1)
    if config.t0:
        start, end = max(start, config.t0), min(end, config.end())
    return start, max(start, min(end, common.now()))


def last_record(config: common.Config, directory, day: str) -> dict | None:
    previous = common.day_of(common.day_start(day) - timedelta(days=1))
    records = common.read_day(directory, previous)
    return records[-1] if records else None


def in_window(records: list[dict], start, end) -> list[dict]:
    return [r for r in records if start <= common.parse_time(r["t"]) < end]


def analyze(config: common.Config, days: list[str]) -> dict:
    windows = fault_windows(config)
    thresholds = config.thresholds
    nodes = {}
    all_days = sorted(set(config.soak_days() + days))
    trend_days = [d for d in all_days if d <= max(days)]
    for name, spec in sorted(config.nodes.items()):
        samples, logs, resources = [], [], []
        expected = 0.0
        for day in days:
            start, end = window_of(config, day)
            expected += (end - start).total_seconds() / SAMPLE_INTERVAL_S
            samples += in_window(common.read_day(config.dir("samples", name), day), start, end)
            logs += in_window(read_logs(config, name, day), start, end)
            resources += in_window(common.read_day(config.dir("samples", name, "resources"), day), start, end)
        baseline = last_record(config, config.dir("samples", name), days[0])
        if config.t0 and baseline and common.parse_time(baseline["t"]) < config.t0 - timedelta(minutes=10):
            baseline = None
        history = []
        for day in trend_days:
            start, end = window_of(config, day)
            history += in_window(common.read_day(config.dir("samples", name, "resources"), day), start, end)
        status = status_analysis(samples, baseline, windows, name)
        status.pop("last", None)
        status["expected"] = int(round(expected))
        resource = resource_analysis(resources, windows, name, spec, thresholds)
        history_values = resource_analysis(history, windows, name, spec, thresholds)["values"]
        resource["trend"] = [t for metric in ("rss_kib", "fds", "sockets")
                             for t in trend(history_values, metric, thresholds)]
        resource.pop("values")
        nodes[name] = {"status": status, "logs": log_analysis(logs, windows, name), "resources": resource,
                       "role": "relay/exit" if spec.get("relay_or_exit") else "plain"}
    server_samples, server_logs = [], []
    for day in days:
        start, end = window_of(config, day)
        server_samples += in_window(common.read_day(config.dir("samples", "server"), day), start, end)
        server_logs += in_window(read_logs(config, "server", day), start, end)
    server = server_analysis(server_samples, last_record(config, config.dir("samples", "server"), days[0]), windows)
    server["logs"] = log_analysis(server_logs, windows, "server")
    probes = []
    for day in days:
        probes += common.read_day(config.dir("samples", "probes"), day)
    disk_stops = [r for r in (common.read_jsonl(config.dir("state", "disk-stop.jsonl"))
                              if config.dir("state", "disk-stop.jsonl").exists() else [])
                  if common.day_of(common.parse_time(r["t"])) in days]
    result = {"days": days, "t0": common.iso(config.t0) if config.t0 else None, "generated": common.iso(common.now()),
              "nodes": nodes, "server": server, "probes": probes,
              "faults": [fault_view(w) for w in windows if w["start"] is None or common.day_of(w["start"]) in days
                         or (w.get("planned_at") and w["planned_at"][:10] in days)],
              "disk": {"free_gib": round(common.free_gib(config.root), 1), "floor_gib": config.min_free_gib,
                       "stops": disk_stops}}
    result["gates"] = gates(config, result)
    return result


def fault_view(window: dict) -> dict:
    return {"id": window["id"], "planned_at": window.get("planned_at"), "action": window.get("action"),
            "start": common.iso(window["start"]) if window.get("start") else None,
            "end": common.iso(window["end"]) if window.get("end") else None, "ok": window.get("ok"),
            "error": window.get("error"), "skipped": window.get("skipped"), "detail": window.get("detail"),
            "interrupted": window.get("interrupted")}


def gate(name: str, passed, evidence: str) -> dict:
    return {"gate": name, "result": "pass" if passed is True else "fail" if passed is False else "incomplete",
            "evidence": evidence}


def gates(config: common.Config, result: dict) -> list[dict]:
    nodes, server = result["nodes"], result["server"]
    out = []
    unplanned = [(n, s) for n, v in nodes.items() for s in v["status"]["starts"] if not s["planned"]]
    unplanned += [("server", s) for s in server["starts"] if not s["planned"]]
    planned_starts = sum(1 for v in nodes.values() for s in v["status"]["starts"] if s["planned"])
    out.append(gate("零人工重启（计划外进程启动 = 0）", not unplanned,
                    f"计划外 {len(unplanned)}：{', '.join(n + '@' + s['t'] for n, s in unplanned) or '无'}；"
                    f"计划内 {planned_starts + sum(1 for s in server['starts'] if s['planned'])}"))
    blocked = [(n, b) for n, v in nodes.items() for b in v["status"]["blocked"]]
    log_blocked = [(n, b) for n, v in nodes.items() for b in v["logs"]["blocked"]]
    uncaused = [(n, b) for n, b in blocked if not b["has_cause"]]
    review = [(n, b) for n, b in blocked if not b["planned"]]
    out.append(gate("每次 blocked 都有已证实原因", not uncaused,
                    f"blocked 段 {len(blocked)}（采样）＋日志 Blocked {len(log_blocked)}；无原因 {len(uncaused)}；"
                    f"计划外待人工确认 {len(review)}"))
    cpu = [(n, v) for n, item in nodes.items() for v in item["resources"]["violations"]]
    missing = sum(item["resources"]["missing"] for item in nodes.values())
    measured = sum(item["resources"]["samples"] - item["resources"]["missing"] for item in nodes.values())
    out.append(gate("空闲 CPU（每线程 < 5 %；中继／出口进程合计与 nft 子进程）", (not cpu) if measured else None,
                    f"{measured} 次测量（缺测 {missing}），超限 {len(cpu)}："
                    + ("; ".join(f"{n} {v['t'][:16]} {v['gate']} = {v['value']}" for n, v in cpu[:8]) or "无")))
    trends = [(n, t) for n, item in nodes.items() for t in item["resources"]["trend"]]
    growing = [(n, t) for n, t in trends if t["verdict"] == "growing"]
    judged = [t for _, t in trends if t["verdict"] != "too short"]
    out.append(gate("RSS／fd／socket 无持续增长", (not growing) if judged else None,
                    f"判定 {len(judged)} 段，增长 {len(growing)}："
                    + ("; ".join(f"{n} {t['metric']} {t['slope_per_day']}/d" for n, t in growing) or "无")))
    sync_bad = [(n, f) for n, v in nodes.items() for f in v["logs"]["sync_failures"]
                if not f["planned"] and not f["reachability"]]
    sync_unplanned = sum(1 for v in nodes.values() for f in v["logs"]["sync_failures"] if not f["planned"])
    out.append(gate("计划外 sync 失败只属于 tunnel／Server 可达性", not sync_bad,
                    f"计划外 sync 失败 {sync_unplanned}，非可达性类 {len(sync_bad)}："
                    + ("; ".join(f"{n} {f['t'][:19]} {f['error']}" for n, f in sync_bad[:8]) or "无")))
    edges = [tuple(e) for e in config.data.get("edges", [])]
    pings = {}
    for record in result["probes"]:
        if record.get("kind") == "ping":
            pings.setdefault((record["src"], record["dst"]), []).append(record)
    bad_edges, missing_edges = [], []
    for day in result["days"]:
        for edge in edges:
            runs = [r for r in pings.get(edge, []) if r["t"][:10] == day]
            if not runs:
                missing_edges.append(f"{day} {edge[0]}>{edge[1]}")
            elif not all(r.get("sent") == 10 and r.get("success") == 10 for r in runs):
                bad_edges.append(f"{day} {edge[0]}>{edge[1]} {runs[-1].get('success')}/{runs[-1].get('sent')}")
    out.append(gate("每日 ping 10/10", False if bad_edges else None if missing_edges else True,
                    f"失败 {bad_edges or '无'}；缺测 {missing_edges or '无'}"))
    coverage = {n: (v["status"]["samples"], v["status"]["expected"]) for n, v in nodes.items()}
    thin = [f"{n} {s}/{e}" for n, (s, e) in coverage.items() if e and s < 0.9 * e]
    out.append(gate("采样覆盖 ≥ 90 %（不是门槛，缺测不填零）", not thin, ", ".join(thin) or "全部 ≥ 90 %"))
    return out


# ---------------------------------------------------------------- rendering

def mark(planned_id: str | None) -> str:
    return f"计划内（{planned_id}）" if planned_id else "**计划外**"


def render(config: common.Config, result: dict, title: str) -> str:
    lines = [f"# {title}", ""]
    days = result["days"]
    index = ""
    if config.t0 and len(days) == 1 and days[0] in config.soak_days():
        index = f"（D{config.soak_days().index(days[0]) + 1}）"
    lines.append(f"- 范围：{days[0]}…{days[-1]} {index}；T0 {result['t0']}；生成 {result['generated']}")
    disk = result["disk"]
    if disk["stops"] or disk["free_gib"] < disk["floor_gib"]:
        lines.append(f"- 🔴 **磁盘剩余 {disk['free_gib']} GiB < {disk['floor_gib']} GiB：写入已停止"
                     f"（{len(disk['stops'])} 次，首次 {disk['stops'][0]['t'] if disk['stops'] else '现在'}），本报告不完整**")
    else:
        lines.append(f"- 磁盘剩余 {disk['free_gib']} GiB（下限 {disk['floor_gib']} GiB）")
    lines += ["", "## 门槛", "", "| 门槛 | 结果 | 依据 |", "| --- | --- | --- |"]
    symbols = {"pass": "✅ 通过", "fail": "❌ 未过", "incomplete": "⚠️ 不完整"}
    for item in result["gates"]:
        lines.append(f"| {item['gate']} | {symbols[item['result']]} | {item['evidence']} |")
    lines += ["", "## 故障注入", ""]
    if result["faults"]:
        lines += ["| id | 计划 | 开始 | 结束 | 结果 | 细节 |", "| --- | --- | --- | --- | --- | --- |"]
        for fault in result["faults"]:
            outcome = "跳过：" + fault["skipped"] if fault.get("skipped") else (
                "✅" if fault["ok"] else f"❌ {fault.get('error') or ''}{'（中断）' if fault.get('interrupted') else ''}")
            detail = json.dumps(fault.get("detail") or {}, ensure_ascii=False)[:200]
            lines.append(f"| {fault['id']} | {fault.get('planned_at') or ''} | {fault.get('start') or ''} | "
                         f"{fault.get('end') or ''} | {outcome} | `{detail}` |")
    else:
        lines.append("无。")
    lines += ["", "## 节点", "",
              "| 节点 | 角色 | 样本 | 不可达 | phase | 进程启动 | 续租 | 最少剩余租约 h | serial 变化 | syncError 样本 |",
              "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for name, item in result["nodes"].items():
        s = item["status"]
        starts = f"{sum(1 for x in s['starts'] if x['planned'])} 计划内／{sum(1 for x in s['starts'] if not x['planned'])} 计划外"
        serials = ", ".join(f"{x['from']}→{x['to']}@{x['t'][11:16]}" for x in s["serials"]) or "—"
        lines.append(f"| {name} | {item['role']} | {s['samples']}/{s['expected']} | {s['unreachable']} | "
                     f"{json.dumps(s['phases'], ensure_ascii=False)} | {starts} | {s['lease_renewals']} | "
                     f"{s['min_lease_left_h']} | {serials} | {json.dumps(s['sync_errors']) if s['sync_errors'] else '—'} |")
    srv = result["server"]
    lines.append("")
    lines.append(f"Server：健康探测 {srv['health_ok']}/{srv['samples']} 成功；进程启动 "
                 + (", ".join(f"{x['t']} {mark(x['planned'])}" for x in srv["starts"]) or "无")
                 + "；失败 " + (", ".join(f"{x['t'][11:16]} {x.get('http') or x.get('error')} {mark(x['planned'])}"
                                         for x in srv["health_failed"][:12]) or "无"))
    lines += ["", "## blocked／撤回／重装／进程启动", ""]
    events = []
    for name, item in result["nodes"].items():
        for b in item["status"]["blocked"]:
            causes = "; ".join(f"{c[0]}: {c[1] or ''}" for c in b["causes"])
            events.append((b["start"], name, f"blocked（采样 {b['samples']} 次，至 {b['end'][11:19]}）", causes, b["planned"]))
        for b in item["logs"]["blocked"]:
            events.append((b["t"], name, f"日志 Blocked：{b['message']}", json.dumps(b["fields"], ensure_ascii=False), b["planned"]))
        for w in item["logs"]["withdrawals"]:
            events.append((w["t"], name, "撤回：" + w["message"], json.dumps(w["fields"], ensure_ascii=False), w["planned"]))
        for w in item["logs"]["reinstalls"]:
            events.append((w["t"], name, "重装：" + w["message"], json.dumps(w["fields"], ensure_ascii=False), w["planned"]))
        for x in item["status"]["starts"]:
            events.append((x["t"], name, f"进程启动 pid {x['previous_pid']}→{x['pid']}（{x['after'][11:19]} 之后）", "", x["planned"]))
    if events:
        lines += ["| 时刻 | 节点 | 事件 | 原因／字段 | 标注 |", "| --- | --- | --- | --- | --- |"]
        for t, name, what, why, tag in sorted(events)[:300]:
            lines.append(f"| {t[:19]} | {name} | {what} | {why.replace('|', '/')[:240]} | {mark(tag)} |")
    else:
        lines.append("无。")
    lines += ["", "## sync 失败（日志）", "", "| 节点 | 计划内 | 计划外·可达性 | 计划外·其他 |", "| --- | --- | --- | --- |"]
    for name, item in result["nodes"].items():
        f = item["logs"]["sync_failures"]
        lines.append(f"| {name} | {sum(1 for x in f if x['planned'])} | "
                     f"{sum(1 for x in f if not x['planned'] and x['reachability'])} | "
                     f"{sum(1 for x in f if not x['planned'] and not x['reachability'])} |")
    lines += ["", "## 错误计数（WARN／ERROR，按类别）", ""]
    for name, item in list(result["nodes"].items()) + [("server", {"logs": srv["logs"]})]:
        logs = item["logs"]
        top = sorted(logs["categories"].items(), key=lambda kv: -kv[1])[:15]
        lines.append(f"- **{name}**：{logs['lines']} 行，{json.dumps(logs['levels'])}" + ("" if top else "；无 WARN／ERROR"))
        for key, count in top:
            lines.append(f"  - {count} × `{key.replace('`', '')}`")
    lines += ["", "## CPU／RSS／fd（每小时 60 s schedstat 窗口，单核 %）", "",
              "| 节点 | 测量 | 最忙线程 中位／最大 | 进程 中位／最大 | 子进程 中位／最大 | RSS MiB 中位／最大 | fd | socket | 超限 |",
              "| --- | --- | --- | --- | --- | --- | --- | --- | --- |"]

    def pair(value, scale=1.0):
        return f"{round(value['median'] / scale, 2)}／{round(value['max'] / scale, 2)}" if value else "—"
    for name, item in result["nodes"].items():
        r = item["resources"]
        lines.append(f"| {name} | {r['samples'] - r['missing']}/{r['samples']}（故障窗口排除 {r['excluded']}） | "
                     f"{pair(r['max_thread_pct'])} | {pair(r['process_pct'])} | {pair(r['children_pct'])} | "
                     f"{pair(r['rss_kib'], 1024)} | {pair(r['fds'])} | {pair(r['sockets'])} | {len(r['violations'])} |")
    lines += ["", "## 趋势（自 T0 起每个进程段的线性拟合）", "",
              "| 节点 | 指标 | pid | 时长 h | 斜率／天 | 7 天外推 | 限值 | 首 24 h → 末 24 h 中位 | 判定 |",
              "| --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for name, item in result["nodes"].items():
        for t in item["resources"]["trend"]:
            lines.append(f"| {name} | {t['metric']} | {t['pid']} | {t['hours']} | {t.get('slope_per_day', '—')} | "
                         f"{t.get('projected', '—')} | {t.get('limit', '—')} | "
                         f"{t.get('first_24h', '—')} → {t.get('last_24h', '—')} | {t['verdict']} |")
    lines += ["", "## 探测", ""]
    pings = [p for p in result["probes"] if p.get("kind") == "ping"]
    if pings:
        lines += ["| 时刻 | 边 | 成功／发送 | 路径节点 | 错误 |", "| --- | --- | --- | --- | --- |"]
        for p in pings:
            lines.append(f"| {p['t'][:16]} | {p['src']}→{p['dst']} | {p.get('success')}/{p.get('sent')} | "
                         f"{p.get('path', '')} | {str(p.get('error') or '')[:120]} |")
    for p in result["probes"]:
        if p.get("kind") == "throughput":
            lines.append(f"- 吞吐 {p['t'][:16]} {p['src']}→{p['dst']}（限速 {p['mbit_limit']} Mbit/s，{p['seconds']} s）："
                         f"{p.get('received_mbit', '—')} Mbit/s，重传 {p.get('retransmits', '—')} {p.get('error') or ''}")
    if not result["probes"]:
        lines.append("无探测记录（缺测）。")
    return "\n".join(lines) + "\n"


def write(config: common.Config, name: str, result: dict, title: str) -> None:
    directory = config.dir("reports")
    directory.mkdir(parents=True, exist_ok=True)
    for suffix, body in ((".json", json.dumps(result, indent=1, ensure_ascii=False, default=str) + "\n"),
                         (".md", render(config, result, title))):
        temporary = directory / f"{name}{suffix}.tmp"
        temporary.write_text(body)
        temporary.replace(directory / f"{name}{suffix}")


def collect_logs(config: common.Config) -> None:
    import logs
    try:
        with common.lock(config, "logs-inner", wait=900):
            logs.collect(config)
    except TimeoutError:
        common.note(config, "report", "log collection lock busy; reporting stored logs only")


def main(argv: list[str]) -> int:
    config, rest = common.config_from_args(argv)
    if rest[:1] == ["probe"]:
        return probe(config)
    if "--no-collect" not in rest:
        collect_logs(config)
    if "--final" in rest:
        days = config.soak_days()
        result = analyze(config, days)
        write(config, "final", result, f"Meshway soak 最终判定（{days[0]}…{days[-1]}）")
    else:
        day = rest[rest.index("--date") + 1] if "--date" in rest else common.day_of(common.now() - timedelta(days=1))
        result = analyze(config, [day])
        write(config, day, result, f"Meshway soak 日报 {day}")
        common.prune(config, common.day_of(common.now()))
    common.note(config, "report", " ".join(f"{g['result']}" for g in result["gates"]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
