#!/usr/bin/env python3
"""Planned fault injection for the soak. Every injection writes a `start`
marker to faults.jsonl before it acts and an `end` marker afterwards — also
when it fails or times out — so the report can label each event planned or
unplanned. Every step has a bounded timeout; the nft faults also leave a
dead-man removal inside the container.

usage:
  fault.py due                 run faults whose time has come (cron, every minute)
  fault.py run ID [--force]    run one fault now (outside the soak window only with --force)
  fault.py recover             close an interrupted fault: run its cleanup, write its end marker
  fault.py list                schedule and recorded state
  (all accept --config PATH and --dry-run)

Physical hosts only get process-level faults; nft rules are only created in a
container's own netns, in the separate table `inet meshway_soak_fault`.
"""
from __future__ import annotations

import json
import sys
import time
import urllib.error
import urllib.request
from datetime import timedelta

import common

LATE = timedelta(minutes=30)
TABLE = "meshway_soak_fault"
DRY = False


class FaultError(Exception):
    pass


# ---------------------------------------------------------------- schedule and markers

def planned_at(config: common.Config, fault: dict):
    hours, minutes = (int(x) for x in fault["at"].split(":"))
    first = common.day_start(common.day_of(config.t0))
    return first + timedelta(days=int(fault["day"]) - 1, hours=hours, minutes=minutes)


def faults_path(config: common.Config):
    return config.dir("faults.jsonl")


def records(config: common.Config) -> list[dict]:
    path = faults_path(config)
    return common.read_jsonl(path) if path.exists() else []


def mark(config: common.Config, record: dict) -> None:
    record = {"t": common.iso(common.now()), **record}
    if DRY:  # a dry run never claims a schedule slot
        print("DRY marker", json.dumps(record, sort_keys=True))
        return
    path = faults_path(config)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as handle:  # markers are written even below the disk floor: they are tiny
        handle.write(json.dumps(record, sort_keys=True) + "\n")


def by_id(config: common.Config) -> dict:
    return {f["id"]: f for f in config.data.get("faults", [])}


def recorded(config: common.Config) -> dict:
    """id → {'start': rec, 'end': rec, 'skipped': rec} (last of each)."""
    out: dict = {}
    for record in records(config):
        out.setdefault(record["id"], {})[record["event"]] = record
    return out


# ---------------------------------------------------------------- helpers

def node_status(config: common.Config, node: str) -> dict | None:
    result = common.run_node(config, node, ["meshway", "--json", "status"], 30)
    try:
        return json.loads(result.out) if result.ok else None
    except json.JSONDecodeError:
        return None


def wait_running(config: common.Config, node: str, started: float, timeout: float, old_pid=None) -> dict:
    """Poll Status until running/ready (and, if given, a new process); seconds since `started`."""
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        status = node_status(config, node)
        if status:
            last = {"phase": status.get("phase"), "dataPlane": status.get("dataPlane"), "pid": status.get("processId")}
            if (status.get("phase") == "running" and status.get("dataPlane") == "ready"
                    and (old_pid is None or status.get("processId") != old_pid)):
                return {"recovered_s": round(time.monotonic() - started, 1), "pid_after": status.get("processId")}
        time.sleep(3)
    raise FaultError(f"{node} not running/ready within {timeout:.0f} s (last {last})")


def wait_health(config: common.Config, started: float, timeout: float) -> dict:
    url = config.server.get("health_url")
    if not url:
        return {}
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=10) as response:
                if response.status == 200:
                    return {"server_healthy_s": round(time.monotonic() - started, 1)}
        except (urllib.error.URLError, OSError):
            pass
        time.sleep(3)
    raise FaultError(f"Server health {url} not 200 within {timeout:.0f} s")


def must(result: common.Result, what: str) -> common.Result:
    if not result.ok:
        raise FaultError(f"{what} failed: {result.tail(300)}")
    return result


def node_exec(config, fault, inner, timeout, stdin=None):
    if DRY:
        print("DRY", fault["node"], common.node_argv(config, fault["node"], inner))
        return common.Result(0, "", "", 0, inner)
    return common.run_node(config, fault["node"], inner, timeout, stdin)


def host_exec(config, host, inner, timeout):
    if DRY:
        print("DRY", host, common.host_argv(config, host, inner))
        return common.Result(0, "", "", 0, inner)
    return common.run_host(config, host, inner, timeout)


def nap(seconds: float) -> None:
    if DRY:
        print("DRY sleep", seconds)
        return
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        time.sleep(min(10.0, max(0.0, end - time.monotonic())))


def template(fault: dict, key: str) -> str:
    command = fault[key]
    if "REPLACE" in command:
        raise FaultError(f"{key} is a placeholder; fill it from the PROD-1 README before the soak")
    return command.format(**fault.get("params", {}))


# ---------------------------------------------------------------- actions (run, cleanup)

def recovery_timeout(fault):
    return float(fault.get("recovery_timeout_s", 300))


def run_restart(config, fault):
    node = config.nodes[fault["node"]]
    before = node_status(config, fault["node"]) if not DRY else {"processId": 0}
    old = (before or {}).get("processId")
    started = time.monotonic()
    action = fault["action"]
    if action == "systemd_restart":
        must(node_exec(config, fault, ["systemctl", "restart", node.get("unit", "meshwayd")], 120), "systemctl restart")
    elif action == "kill9":
        must(node_exec(config, fault, common.shell(f"kill -9 $({common.pid_expr(config, fault['node'])})"), 30), "kill -9")
    elif action == "docker_restart":
        must(host_exec(config, node["host"], ["docker", "restart", "-t", "30", node["container"]], 120), "docker restart")
    detail = {"pid_before": old}
    if not DRY:
        detail.update(wait_running(config, fault["node"], started, recovery_timeout(fault), old_pid=old))
    return detail


def run_server_stop(config, fault):
    server = config.server
    started = time.monotonic()
    must(host_exec(config, server["host"], ["docker", "stop", "-t", "30", server["container"]], 120), "docker stop")
    try:
        nap(float(fault.get("duration_s", 1800)))
    finally:
        must(host_exec(config, server["host"], ["docker", "start", server["container"]], 120), "docker start")
    return {"stopped_s": round(time.monotonic() - started, 1),
            **(wait_health(config, time.monotonic(), recovery_timeout(fault)) if not DRY else {})}


def cleanup_server_stop(config, fault):
    host_exec(config, config.server["host"], ["docker", "start", config.server["container"]], 120)


def run_command(config, fault):
    command = template(fault, "command")
    started = time.monotonic()
    result = host_exec(config, fault["host"], common.shell(command), float(fault.get("timeout_s", 900)))
    detail = {"rc": result.rc, "output_tail": result.out.strip()[-800:], "ms": result.ms}
    must(result, "command")
    if fault.get("wait_health", True) and not DRY:
        detail.update(wait_health(config, started, recovery_timeout(fault)))
    return detail


def cleanup_command(config, fault):
    if fault.get("cleanup_command"):
        host_exec(config, fault["host"], common.shell(template(fault, "cleanup_command")), 600)


def nft_rules(fault, addresses) -> str:
    if fault["match"] == "udp":
        rule = "meta l4proto udp"
        hooks = ("input", "output", "forward")
        lo = {"input": "iif lo accept; ", "output": "oif lo accept; ", "forward": ""}
    elif fault["match"] == "tcp-to-server":
        v4 = [a for a in addresses if ":" not in a]
        v6 = [a for a in addresses if ":" in a]
        if not v4 and not v6:
            raise FaultError("no Server address resolved")
        parts = []
        if v4:
            parts.append("ip daddr { %s } meta l4proto tcp drop; " % ", ".join(v4))
        if v6:
            parts.append("ip6 daddr { %s } meta l4proto tcp drop; " % ", ".join(v6))
        return ("table inet %s {\n chain output { type filter hook output priority -150; policy accept; %s}\n}\n"
                % (TABLE, "".join(parts)))
    else:
        raise FaultError(f"unknown match {fault['match']!r}")
    chains = "".join(" chain %s { type filter hook %s priority -150; policy accept; %s%s drop; }\n"
                     % (hook, hook, lo[hook], rule) for hook in hooks)
    return "table inet %s {\n%s}\n" % (TABLE, chains)


def resolve_server(config, fault) -> list[str]:
    if fault.get("addresses"):
        return list(fault["addresses"])
    host = fault.get("resolve") or config.server.get("health_url", "").split("/")[2].split(":")[0]
    result = must(node_exec(config, fault, common.shell(f"getent ahosts {host} | awk '{{print $1}}' | sort -u"), 30),
                  "resolve")
    return [line.strip() for line in result.out.splitlines() if line.strip()]


def run_nft_drop(config, fault):
    node = config.nodes[fault["node"]]
    if not node.get("container"):
        raise FaultError("nft faults only run inside a container netns")
    duration = float(fault.get("duration_s", 300))
    addresses = resolve_server(config, fault) if fault["match"] == "tcp-to-server" else []
    rules = nft_rules(fault, addresses)
    # The dead man removes the table even if this process or the collector's connection dies.
    must(host_exec(config, node["host"], ["docker", "exec", "-d", node["container"], "sh", "-c",
                                          f"sleep {int(duration) + 120}; nft delete table inet {TABLE}"], 30),
         "dead-man")
    must(node_exec(config, fault, ["nft", "-f", "-"], 30, stdin=rules), "nft add")
    started = time.monotonic()
    try:
        listed = node_exec(config, fault, ["nft", "list", "table", "inet", TABLE], 30)
        nap(duration)
    finally:
        removed = node_exec(config, fault, ["nft", "delete", "table", "inet", TABLE], 30)
    must(removed, "nft delete")
    detail = {"addresses": addresses, "installed": listed.ok, "dropped_s": round(time.monotonic() - started, 1)}
    if not DRY:
        detail.update(wait_running(config, fault["node"], time.monotonic(), recovery_timeout(fault)))
    return detail


def cleanup_nft_drop(config, fault):
    result = node_exec(config, fault, ["nft", "delete", "table", "inet", TABLE], 30)
    if not result.ok and "No such file" not in result.err:
        raise FaultError(f"nft delete failed: {result.tail()}")


def run_exit_profile(config, fault):
    duration = int(fault.get("duration_s", 600))
    url = fault.get("url", "https://www.example.com/")
    must(node_exec(config, fault, ["meshway", "--json", "exit", fault["profile"]], 90), "meshway exit")
    detail = {}
    try:
        status = node_status(config, fault["node"]) if not DRY else {}
        routing = ((status or {}).get("netmap") or {}).get("routing") or {}
        detail["applied_profile"] = routing.get("appliedProfile")
        loop = (f"end=$(( $(date +%s) + {duration} )); ok=0; n=0; "
                f"while [ $(date +%s) -lt $end ]; do n=$((n+1)); "
                f"curl -fsS -m 10 -o /dev/null '{url}' && ok=$((ok+1)); sleep 2; done; echo \"@@curl $ok $n\"")
        result = node_exec(config, fault, common.shell(loop), duration + 60)
        for line in result.out.splitlines():
            if line.startswith("@@curl "):
                ok, total = line.split()[1:3]
                detail.update(curl_ok=int(ok), curl_total=int(total))
    finally:
        cleared = node_exec(config, fault, ["meshway", "--json", "exit", "--default"], 90)
    must(cleared, "meshway exit --default")
    if detail.get("applied_profile") not in (None, fault["profile"]) or (
            "curl_total" in detail and detail["curl_ok"] == 0):
        raise FaultError(f"exit profile not effective: {detail}")
    return detail


def cleanup_exit_profile(config, fault):
    node_exec(config, fault, ["meshway", "--json", "exit", "--default"], 90)


ACTIONS = {
    "systemd_restart": (run_restart, None),
    "kill9": (run_restart, None),
    "docker_restart": (run_restart, None),
    "server_stop": (run_server_stop, cleanup_server_stop),
    "command": (run_command, cleanup_command),
    "nft_drop": (run_nft_drop, cleanup_nft_drop),
    "exit_profile": (run_exit_profile, cleanup_exit_profile),
}


def affects(fault) -> list[str]:
    if "affects" in fault:
        return fault["affects"]
    if fault["action"] in ("server_stop", "command"):
        return ["server", "*"]
    return [fault["node"]]


def execute(config: common.Config, fault: dict, planned=None) -> bool:
    run, _ = ACTIONS[fault["action"]]
    mark(config, {"event": "start", "id": fault["id"], "action": fault["action"],
                  "planned_at": common.iso(planned) if planned else None, "affects": affects(fault),
                  "grace_min": fault.get("grace_min", 15), "params": fault.get("params", {}), "dry": DRY})
    ok, detail, error = False, {}, None
    try:
        detail = run(config, fault) or {}
        ok = True
    except FaultError as failure:
        error = str(failure)
    except Exception as failure:  # always close the marker
        error = f"{type(failure).__name__}: {failure}"
    finally:
        mark(config, {"event": "end", "id": fault["id"], "ok": ok, "detail": detail, "error": error,
                      "interrupted": False})
    common.note(config, "fault", f"{fault['id']} ok={ok} {error or ''}")
    return ok


def recover(config: common.Config) -> list[str]:
    """Faults with a start and no later end were interrupted: clean up, then close them."""
    closed = []
    faults = by_id(config)
    for fid, events in recorded(config).items():
        start, end = events.get("start"), events.get("end")
        if not start or (end and end["t"] >= start["t"]):
            continue
        fault = faults.get(fid)
        error = None
        try:
            cleanup = ACTIONS.get(fault["action"], (None, None))[1] if fault else None
            if cleanup:
                cleanup(config, fault)
        except Exception as failure:
            error = f"cleanup: {failure}"
        mark(config, {"event": "end", "id": fid, "ok": False, "detail": {}, "error": error or "interrupted",
                      "interrupted": True})
        closed.append(fid)
    return closed


def due(config: common.Config) -> int:
    recover(config)
    state = recorded(config)
    moment = common.now()
    for fault in sorted(config.data.get("faults", []), key=lambda f: planned_at(config, f)):
        if fault["id"] in state:
            continue
        planned = planned_at(config, fault)
        if moment < planned:
            continue
        if moment > planned + LATE:
            mark(config, {"event": "skipped", "id": fault["id"], "planned_at": common.iso(planned),
                          "reason": f"missed: due {common.iso(planned)}, collector ran at {common.iso(moment)}"})
            continue
        execute(config, fault, planned)
        return 0  # one fault per invocation; the next minute picks up the next one
    return 0


def main(argv: list[str]) -> int:
    global DRY
    config, rest = common.config_from_args(argv)
    if "--dry-run" in rest:
        DRY = True
        rest.remove("--dry-run")
    force = "--force" in rest
    rest = [a for a in rest if a != "--force"]
    command = rest[0] if rest else "list"
    with common.lock(config, "fault-inner", wait=0 if command == "due" else 5):
        if command == "due":
            if not config.t0:
                return 0
            return due(config)
        if command == "recover":
            print(json.dumps(recover(config)))
            return 0
        if command == "run":
            fault = by_id(config).get(rest[1] if len(rest) > 1 else "")
            if not fault:
                print("unknown fault id", file=sys.stderr)
                return 2
            moment = common.now()
            inside = config.t0 and config.t0 <= moment < config.end()
            if not inside and not force:
                print("outside the soak window; pass --force", file=sys.stderr)
                return 2
            recover(config)
            return 0 if execute(config, fault, planned_at(config, fault) if config.t0 else None) else 1
        if command == "list":
            state = recorded(config)
            for fault in config.data.get("faults", []):
                events = state.get(fault["id"], {})
                print(f"{fault['id']:28} {common.iso(planned_at(config, fault)) if config.t0 else '?':28} "
                      f"{fault['action']:16} {','.join(sorted(events)) or 'pending'}")
            return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
