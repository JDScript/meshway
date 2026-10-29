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
  fault.py check               gates answer with the scheduled forms; kill -9 targets have a restart policy
  (all accept --config PATH and --dry-run)

Physical hosts only get process-level faults; nft rules are only created in a
container's own netns, in the separate table `inet meshway_soak_fault`.
"""
from __future__ import annotations

import json
import sys
import time
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

def node_process(config: common.Config, node: str) -> tuple[dict | None, dict | None]:
    """Status and the daemon's process identity (`@@proc`: pid and start time)."""
    result = common.node_call(config, node, "status", timeout=60)
    proc, body = common.proc_line(result.out)
    try:
        return (json.loads(body) if result.ok else None), proc
    except json.JSONDecodeError:
        return None, proc


def node_status(config: common.Config, node: str) -> dict | None:
    return node_process(config, node)[0]


def replaced(old: dict | None, status: dict, proc: dict | None) -> bool:
    """A new daemon process: another pid, or the same pid started later (a restarted container reuses its pids)."""
    if old is None:
        return True
    if status.get("processId") != old.get("pid"):
        return True
    return bool(proc and old.get("start") is not None and proc.get("start") is not None
                and proc["start"] != old["start"])


def wait_running(config: common.Config, node: str, started: float, timeout: float, old: dict | None = None) -> dict:
    """Poll Status until running/ready (and, if `old` names the previous process, a new one); seconds since `started`."""
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        status, proc = node_process(config, node)
        if status:
            last = {"phase": status.get("phase"), "dataPlane": status.get("dataPlane"), "pid": status.get("processId"),
                    "start": (proc or {}).get("start")}
            if status.get("phase") == "running" and status.get("dataPlane") == "ready" and replaced(old, status, proc):
                return {"recovered_s": round(time.monotonic() - started, 1), "pid_after": status.get("processId"),
                        "start_after": (proc or {}).get("start")}
        time.sleep(3)
    raise FaultError(f"{node} not running/ready within {timeout:.0f} s (last {last})")


def wait_health(config: common.Config, started: float, timeout: float) -> dict:
    url = config.server.get("health_url")
    if not url:
        return {}
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if common.health(url, 10).get("http") == 200:
            return {"server_healthy_s": round(time.monotonic() - started, 1)}
        time.sleep(3)
    raise FaultError(f"Server health {url} not 200 within {timeout:.0f} s")


def must(result: common.Result, what: str) -> common.Result:
    if not result.ok:
        raise FaultError(f"{what} failed: {result.tail(300)}")
    return result


def gate(config, target: str, words: list, timeout: float) -> common.Result:
    """One gate form on the target's host (target = node name or `server`)."""
    host = common.host_of(config, target)
    if DRY:
        print("DRY", host, common.gate_argv(config, host, words))
        return common.Result(0, "", "", 0, words)
    return common.call(config, host, words, timeout)


def nap(seconds: float) -> None:
    if DRY:
        print("DRY sleep", seconds)
        return
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        time.sleep(min(10.0, max(0.0, end - time.monotonic())))


# ---------------------------------------------------------------- actions (run, cleanup)

def recovery_timeout(fault):
    return float(fault.get("recovery_timeout_s", 300))


def restart_policy(config, node) -> str:
    """The policy that brings a killed daemon back: the container's Docker policy or the unit's Restart=."""
    result = gate(config, node, ["inspect", node], 30)
    if DRY:
        return "dry"
    if config.nodes[node].get("container"):
        parts = result.out.strip().rpartition("} ")[2].split(" ")
        return parts[1] if len(parts) > 1 else "?"
    for line in result.out.splitlines():
        if line.startswith("Restart="):
            return line.split("=", 1)[1]
    return "?"


RESTARTING = {"unless-stopped", "always", "on-failure", "on-abnormal", "dry"}


def run_restart(config, fault):
    node = fault["node"]
    before, proc = node_process(config, node) if not DRY else ({"processId": 0}, None)
    old = {"pid": (before or {}).get("processId"), "start": (proc or {}).get("start")}
    detail = {"pid_before": old["pid"], "start_before": old["start"]}
    action = fault["action"]
    if action == "kill9":
        detail["restart_policy"] = restart_policy(config, node)
        if detail["restart_policy"] not in RESTARTING:
            raise FaultError(f"restart policy {detail['restart_policy']!r} would leave {node} down; not killing")
    started = time.monotonic()
    form = {"systemd_restart": "restart", "kill9": "kill9", "docker_restart": "docker-restart"}[action]
    result = must(gate(config, node, [form, node], 130), form)
    for line in result.out.splitlines():
        if line.startswith("@@killed "):
            detail["killed_host_pid"] = int(line.split()[1])
    if not DRY:
        detail.update(wait_running(config, node, started, recovery_timeout(fault), old=old))
    return detail


def run_server_stop(config, fault):
    started = time.monotonic()
    must(gate(config, "server", ["server-stop"], 130), "server-stop")
    try:
        nap(float(fault.get("duration_s", 1800)))
    finally:
        must(gate(config, "server", ["server-start"], 130), "server-start")
    return {"stopped_s": round(time.monotonic() - started, 1),
            **(wait_health(config, time.monotonic(), recovery_timeout(fault)) if not DRY else {})}


def cleanup_server_start(config, fault):
    gate(config, "server", ["server-start"], 130)


def run_server_upgrade(config, fault):
    version = fault.get("version", "")
    started = time.monotonic()
    result = gate(config, "server", ["server-upgrade", version], float(fault.get("timeout_s", 1500)))
    detail = {"version": version, "rc": result.rc, "output_tail": result.out.strip()[-800:]}
    must(result, "server-upgrade")
    if not DRY:
        detail.update(wait_health(config, started, recovery_timeout(fault)))
    return detail


def run_server_rotate_key(config, fault):
    started = time.monotonic()
    result = gate(config, "server", ["server-rotate-key"], float(fault.get("timeout_s", 600)))
    detail = {"rc": result.rc, "output_tail": result.out.strip()[-800:]}
    must(result, "server-rotate-key")
    if not DRY:
        detail.update(wait_health(config, started, recovery_timeout(fault)))
    return detail


def run_nft_drop(config, fault):
    node = fault["node"]
    if not config.nodes[node].get("container"):
        raise FaultError("nft faults only run inside a container netns")
    duration = int(fault.get("duration_s", 300))
    listed = must(gate(config, node, ["nft-drop", node, fault["match"], str(duration)], 120), "nft-drop")
    started = time.monotonic()
    try:
        nap(duration)
    finally:
        cleared = gate(config, node, ["nft-clear", node], 60)
    must(cleared, "nft-clear")
    detail = {"match": fault["match"], "rules": listed.out.strip()[-600:], "dropped_s": round(time.monotonic() - started, 1)}
    if not DRY:
        detail.update(wait_running(config, node, time.monotonic(), recovery_timeout(fault)))
    return detail


def cleanup_nft_drop(config, fault):
    result = gate(config, fault["node"], ["nft-clear", fault["node"]], 60)
    if not result.ok and "No such file" not in result.out + result.err:
        raise FaultError(f"nft-clear failed: {result.tail()}")


def run_exit_profile(config, fault):
    node = fault["node"]
    duration = int(fault.get("duration_s", 600))
    must(gate(config, node, ["exit-select", node, fault["profile"]], 100), "exit-select")
    detail = {"node": node, "profile": fault["profile"]}
    try:
        status = node_status(config, node) if not DRY else {}
        routing = ((status or {}).get("netmap") or {}).get("routing") or {}
        detail["applied_profile"] = routing.get("appliedProfile")
        result = gate(config, node, ["curl-loop", node, str(duration)], duration + 90)
        for line in result.out.splitlines():
            if line.startswith("@@curl "):
                ok, total = line.split()[1:3]
                detail.update(curl_ok=int(ok), curl_total=int(total))
    finally:
        cleared = gate(config, node, ["exit-default", node], 100)
    must(cleared, "exit-default")
    if detail.get("applied_profile") not in (None, fault["profile"]) or (
            "curl_total" in detail and detail["curl_ok"] == 0):
        raise FaultError(f"exit profile not effective: {detail}")
    return detail


def cleanup_exit_profile(config, fault):
    gate(config, fault["node"], ["exit-default", fault["node"]], 100)


ACTIONS = {
    "systemd_restart": (run_restart, None),
    "kill9": (run_restart, None),
    "docker_restart": (run_restart, None),
    "server_stop": (run_server_stop, cleanup_server_start),
    "server_upgrade": (run_server_upgrade, None),  # an interrupted upgrade is for a person (install.md rollback)
    "server_rotate_key": (run_server_rotate_key, cleanup_server_start),
    "nft_drop": (run_nft_drop, cleanup_nft_drop),
    "exit_profile": (run_exit_profile, cleanup_exit_profile),
}


def affects(fault) -> list[str]:
    if "affects" in fault:
        return fault["affects"]
    if fault["action"].startswith("server_"):
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


def check(config: common.Config) -> int:
    """Before the soak: every host's gate answers self-test with the fault forms the schedule needs, and every
    kill -9 target has a policy that brings it back."""
    problems = []
    for host in sorted(config.hosts):
        expected = common.gate_spec(config, host)
        if not expected["nodes"] and "server" not in expected:
            continue
        result = common.call(config, host, ["self-test"], 60)
        try:
            answer = json.loads(result.out)
        except json.JSONDecodeError:
            problems.append(f"{host}: gate did not answer self-test: {result.tail(200)}")
            continue
        if answer.get("faults") != expected["faults"] or answer.get("nodes") != sorted(expected["nodes"]):
            problems.append(f"{host}: gate spec differs from config (re-render and reinstall it): {answer}")
        if not answer.get("sudo") and expected["sudo"]:
            problems.append(f"{host}: sudo -n is not available")
        print(f"{host}: gate ok, nodes {answer.get('nodes')}, faults {answer.get('faults')}")
    for fault in config.data.get("faults", []):
        if fault["action"] == "kill9":
            policy = restart_policy(config, fault["node"])
            print(f"{fault['node']}: restart policy {policy}")
            if policy not in RESTARTING:
                problems.append(f"{fault['node']}: restart policy {policy!r} would leave it down after kill -9")
    for problem in problems:
        print("PROBLEM", problem)
    return 1 if problems else 0


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
        if command == "check":
            return check(config)
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
    try:
        sys.exit(main(sys.argv[1:]))
    except TimeoutError as busy:  # another fault (or a manual run) holds the lock
        print(busy, file=sys.stderr)
        sys.exit(0 if sys.argv[1:2] == ["due"] or "due" in sys.argv else 75)
