#!/usr/bin/env python3
"""Every 5 minutes: each node's `meshway --json status` (with the daemon's PID
and start time) and the Server's health → samples/<node>/<UTC day>.jsonl.

usage: sample.py [--config PATH]
"""
from __future__ import annotations

import json
import sys
import time

import common

STATUS_TIMEOUT = 60


def node_sample(config: common.Config, node: str) -> dict:
    at = common.now()
    result = common.node_call(config, node, "status", timeout=STATUS_TIMEOUT)
    proc, body = common.proc_line(result.out)
    record = {"t": common.iso(at), "node": node, "rc": result.rc, "ms": result.ms, "proc": proc, "status": None}
    try:
        record["status"] = json.loads(body) if body.strip() else None
    except json.JSONDecodeError:
        record["error"] = "status is not JSON: " + body.strip()[:200]
    if record["status"] is None and "error" not in record:
        record["error"] = result.tail()
    return record


def inspect(config: common.Config, target: str) -> dict:
    """`inspect <container target>` → state, start, restart count and policy."""
    result = common.node_call(config, target, "inspect", timeout=30)
    if not result.ok:
        return {"error": result.tail(200)}
    return parse_inspect(result.out)


def parse_inspect(text: str) -> dict:
    state_text, _, rest = text.strip().rpartition("} ")
    try:
        state = json.loads(state_text + "}")
        restarts, policy, image = (rest.split(" ", 2) + ["", ""])[:3]
        return {"status": state.get("Status"), "startedAt": state.get("StartedAt"), "pid": state.get("Pid"),
                "restartCount": int(restarts), "restartPolicy": policy, "image": image}
    except (ValueError, json.JSONDecodeError):
        return {"error": text.strip()[:200]}


def server_sample(config: common.Config) -> dict:
    server = config.server
    at = common.now()
    record = {"t": common.iso(at), "node": "server", "health": None, "container": None}
    url = server.get("health_url")
    if url:
        started = time.monotonic()
        health = {"url": url}
        health.update(common.health(url, 15))
        health["ms"] = int((time.monotonic() - started) * 1000)
        health["ok"] = health.get("http") == 200
        record["health"] = health
    if server.get("container"):
        record["container"] = inspect(config, "server")
    return record


def main(argv: list[str]) -> int:
    config, _ = common.config_from_args(argv)
    names = sorted(config.nodes)
    records = common.parallel(lambda name: node_sample(config, name), names)
    records.append(server_sample(config))
    ok = True
    for record in records:
        ok = common.append_by_day(config, config.dir("samples", record["node"]), [record]) and ok
    failed = [r["node"] for r in records if r["node"] != "server" and r["status"] is None]
    common.note(config, "sample", f"{len(records)} samples, unreachable {failed or 'none'}, written {ok}")
    return 0 if ok else 3


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
