#!/usr/bin/env python3
"""Every 5 minutes: each node's `meshway --json status` (with the daemon's PID
and start time) and the Server's health → samples/<node>/<UTC day>.jsonl.

usage: sample.py [--config PATH]
"""
from __future__ import annotations

import json
import sys
import time
import urllib.error
import urllib.request

import common

STATUS_TIMEOUT = 60


def node_sample(config: common.Config, node: str) -> dict:
    at = common.now()
    result = common.run_node(config, node, common.shell(common.proc_script(config, node) + "; meshway --json status"),
                             STATUS_TIMEOUT)
    proc, body = common.proc_line(result.out)
    record = {"t": common.iso(at), "node": node, "rc": result.rc, "ms": result.ms, "proc": proc, "status": None}
    try:
        record["status"] = json.loads(body) if body.strip() else None
    except json.JSONDecodeError:
        record["error"] = "status is not JSON: " + body.strip()[:200]
    if record["status"] is None and "error" not in record:
        record["error"] = result.tail()
    return record


def server_sample(config: common.Config) -> dict:
    server = config.server
    at = common.now()
    record = {"t": common.iso(at), "node": "server", "health": None, "container": None}
    url = server.get("health_url")
    if url:
        started = time.monotonic()
        health = {"url": url}
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "meshway-soak"}),
                                        timeout=15) as response:
                health["http"] = response.status
                response.read(4096)
        except urllib.error.HTTPError as error:
            health["http"] = error.code
        except (urllib.error.URLError, OSError) as error:
            health["error"] = str(getattr(error, "reason", error))[:200]
        health["ms"] = int((time.monotonic() - started) * 1000)
        health["ok"] = health.get("http") == 200
        record["health"] = health
    if server.get("container"):
        result = common.run_host(config, server["host"],
                                 ["docker", "inspect", "-f", "{{json .State}} {{.RestartCount}} {{.Config.Image}}",
                                  server["container"]], 30)
        if result.ok:
            state_text, _, rest = result.out.strip().rpartition("} ")
            try:
                state = json.loads(state_text + "}")
                restarts, _, image = rest.partition(" ")
                record["container"] = {"status": state.get("Status"), "startedAt": state.get("StartedAt"),
                                       "pid": state.get("Pid"), "restartCount": int(restarts), "image": image}
            except (ValueError, json.JSONDecodeError):
                record["container"] = {"error": result.out.strip()[:200]}
        else:
            record["container"] = {"error": result.tail(200)}
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
