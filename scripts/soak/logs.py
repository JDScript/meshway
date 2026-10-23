#!/usr/bin/env python3
"""Hourly: meshwayd / Server log increments → logs/<node>/<UTC day>.jsonl,
compressed to .jsonl.zst once the day is past.

- systemd nodes: `journalctl -u <unit> -o json --after-cursor <cursor>`; the
  cursor of the last stored entry is kept in state/logs.json, so a missed or
  interrupted run continues where the previous one stopped.
- container nodes and the Server: `docker logs --timestamps --since S --until U`
  with U = now − 30 s (clock skew margin); the next run starts at U + 1 ns.

Each run reads at most MAX_LINES lines per source; the rest follows in the
next run (the cursor / since only advances over what was stored). A crash
between appending and saving the cursor repeats at most one batch; the report
removes exact duplicates.

Possible loss is recorded in state/log-gaps.jsonl (the daily report lists it as 可能缺行): a journal cursor that
can no longer be read (vacuumed; reading resumes from the last stored time), or a docker log that keeps none of
the lines of the last window that had any (rotated past our window end).

All reads go through the host's gate (gate.py).

usage: logs.py [--config PATH]
"""
from __future__ import annotations

import json
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone

import common

MAX_LINES = 200_000
TIMEOUT = 300
LAG = timedelta(seconds=30)


def stream(argv: list[str], limit: int, timeout: float) -> tuple[list[str], bool, str]:
    """Read at most `limit` lines; returns (lines, truncated, error)."""
    lines: list[str] = []
    truncated = False
    started = time.monotonic()
    try:
        process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    except OSError as error:
        return [], False, str(error)
    try:
        assert process.stdout is not None
        for raw in process.stdout:
            lines.append(raw.decode(errors="replace").rstrip("\n"))
            if len(lines) >= limit or time.monotonic() - started > timeout:
                truncated = True
                break
    finally:
        if truncated:
            process.kill()
        try:
            _, err = process.communicate(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            _, err = process.communicate()
    error = ""
    if not truncated and process.returncode not in (0, None):
        error = f"rc {process.returncode}: {err.decode(errors='replace').strip()[-300:]}"
    return lines, truncated, error


def journal_entry(line: str) -> dict | None:
    try:
        entry = json.loads(line)
    except json.JSONDecodeError:
        return None
    message = entry.get("MESSAGE")
    if isinstance(message, list):  # journald encodes non-UTF-8 messages as byte arrays
        message = bytes(message).decode(errors="replace")
    micros = int(entry.get("__REALTIME_TIMESTAMP", "0"))
    moment = datetime.fromtimestamp(micros / 1e6, timezone.utc)
    return {"t": common.iso(moment), "pid": int(entry.get("_PID", "0") or 0),
            "boot": (entry.get("_BOOT_ID") or "")[:8], "prio": int(entry.get("PRIORITY", "6") or 6),
            "sys": entry.get("_PID") == "1" or entry.get("SYSLOG_IDENTIFIER") == "systemd",
            "msg": message or "", "cursor": entry.get("__CURSOR")}


def gap(config: common.Config, name: str, reason: str, since: str) -> None:
    """Record that lines may be missing (rotation / vacuum) for the report."""
    common.append_jsonl(config, config.dir("state", "log-gaps.jsonl"),
                        [{"t": common.iso(common.now()), "node": name, "since": since, "reason": reason}])


def collect_journal(config: common.Config, node: str, state: dict) -> dict:
    start = config.t0 or (common.now() - timedelta(hours=1))
    since_words = ["--since", str(int(start.timestamp()))]
    if state.get("cursor"):
        words = ["--after-cursor", state["cursor"]]
    else:
        words = since_words
    argv = common.gate_argv(config, config.nodes[node]["host"], ["journal", node, *words])
    lines, truncated, error = stream(argv, MAX_LINES, TIMEOUT)
    if error and not lines and state.get("cursor") and state.get("last_t"):
        # The cursor's entry was vacuumed: continue from the last stored time; the report removes the overlap.
        gap(config, node, f"journal cursor no longer readable ({error[:120]})", state["last_t"])
        resume = str(int(common.parse_time(state["last_t"]).timestamp()))
        argv = common.gate_argv(config, config.nodes[node]["host"], ["journal", node, "--since", resume])
        lines, truncated, error = stream(argv, MAX_LINES, TIMEOUT)
    records = [r for r in (journal_entry(line) for line in lines) if r]
    if error and not records:
        return {"error": error, "lines": 0}
    if not common.append_by_day(config, config.dir("logs", node), [{k: v for k, v in r.items() if k != "cursor"}
                                                                  for r in records]):
        return {"error": "disk floor reached; nothing stored", "lines": 0}
    if records:
        state["cursor"] = records[-1]["cursor"]
        state["last_t"] = records[-1]["t"]
    state["at"] = common.iso(common.now())
    return {"lines": len(records), "truncated": truncated, "error": error or None}


def to_ns(moment: datetime) -> int:
    whole = moment.replace(microsecond=0)
    return int(whole.timestamp()) * 1_000_000_000 + moment.microsecond * 1000


def from_ns(ns: int) -> datetime:
    return datetime.fromtimestamp(ns // 1_000_000_000, timezone.utc).replace(microsecond=ns % 1_000_000_000 // 1000)


def docker_since(ns: int) -> str:
    return f"{ns // 1_000_000_000}.{ns % 1_000_000_000:09d}"


def stamp_ns(stamp: str) -> int:
    whole = common.parse_time(stamp).replace(microsecond=0)
    fraction = stamp.rstrip("Z").partition(".")[2] if "." in stamp else ""
    digits = "".join(c for c in fraction if c.isdigit())[:9].ljust(9, "0")
    return int(whole.timestamp()) * 1_000_000_000 + int(digits)


def docker_line(line: str) -> dict | None:
    stamp, _, message = line.partition(" ")
    try:
        moment = common.parse_time(stamp)
    except ValueError:
        return None
    return {"t": stamp if stamp.endswith("Z") else common.iso(moment), "msg": message}


def collect_docker(config: common.Config, name: str, state: dict) -> dict:
    host = common.host_of(config, name)
    until_ns = to_ns(common.now() - LAG)
    if "since_ns" in state:
        since_ns = state["since_ns"]
    else:
        start = config.t0 or (common.now() - timedelta(hours=1))
        since_ns = to_ns(start)
    if since_ns > until_ns:
        return {"lines": 0}
    result: dict = {}
    previous = state.get("previous")  # [since_ns, until_ns, lines stored from that window]
    if previous and previous[2] > 0:
        # Rotation check: if docker no longer keeps any line of the previous window, its oldest kept line is newer
        # than our window start and lines in between may be gone (a partly kept window loses nothing new).
        probe = common.run(common.gate_argv(config, host, ["docker-logs-count", name, docker_since(previous[0]),
                                                           docker_since(previous[1])]), 120)
        counted = [line.split()[1] for line in probe.out.splitlines() if line.startswith("@@count ")]
        if probe.ok and counted == ["0"]:
            gap(config, name, f"docker keeps none of the {previous[2]} lines of the previous window (rotated)",
                common.iso(from_ns(previous[1])))
            result["gap"] = True
    argv = common.gate_argv(config, host, ["docker-logs", name, docker_since(since_ns), docker_since(until_ns)])
    lines, truncated, error = stream(argv, MAX_LINES, TIMEOUT)
    records = [r for r in (docker_line(line) for line in lines) if r]
    if error and not records:
        return {"error": error, "lines": 0, **result}
    if not common.append_by_day(config, config.dir("logs", name), records):
        return {"error": "disk floor reached; nothing stored", "lines": 0}
    if truncated and records:
        # Continue right after the last stored line (docker stamps carry nanoseconds).
        state["since_ns"] = stamp_ns(records[-1]["t"]) + 1
    else:
        state["since_ns"] = until_ns + 1
    if records:  # the reference for the next rotation check is the last window that had lines
        state["previous"] = [since_ns, stamp_ns(records[-1]["t"]) if truncated else until_ns, len(records)]
    state["at"] = common.iso(common.now())
    return {"lines": len(records), "truncated": truncated, "error": error or None, **result}


def sources(config: common.Config) -> list[tuple[str, str]]:
    """(name, kind) for every log source."""
    out = []
    for name, spec in sorted(config.nodes.items()):
        out.append((name, "docker" if spec.get("container") else "journal"))
    if config.server.get("container") and config.server.get("logs", True):
        out.append(("server", "docker"))
    return out


def collect(config: common.Config) -> dict:
    """One collection pass over all sources; returns per-source results. Caller holds the `logs` lock."""
    state = common.read_state(config, "logs.json")

    def one(item):
        name, kind = item
        source_state = dict(state.get(name, {}))
        try:
            if kind == "journal":
                result = collect_journal(config, name, source_state)
            else:
                result = collect_docker(config, name, source_state)
        except Exception as error:  # one source never stops the others
            result = {"error": f"{type(error).__name__}: {error}", "lines": 0}
        return name, source_state, result

    results = {}
    for name, source_state, result in common.parallel(one, sources(config)):
        state[name] = source_state
        results[name] = result
    common.write_state(config, "logs.json", state)
    cutoff = common.day_of(common.now() - timedelta(minutes=5))
    for name, _ in sources(config):
        try:
            common.compress_before(config.dir("logs", name), cutoff)
        except OSError as error:
            results[name]["compress_error"] = str(error)
    return results


def main(argv: list[str]) -> int:
    config, _ = common.config_from_args(argv)
    with common.lock(config, "logs-inner", wait=600):
        results = collect(config)
    common.note(config, "logs", json.dumps(results, sort_keys=True))
    return 0 if all(not r.get("error") for r in results.values()) else 3


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
