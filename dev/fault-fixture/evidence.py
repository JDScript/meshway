"""Evidence of the `fault/throttle` row: parses the Nodes' Status samples
(`<role>/status.jsonl`), the daemons' JSON logs, A's flow log and the
supervisor's per-thread CPU samples, and judges every stage's gate. Pure
functions of the report directory and the manifest; `runtime.py` owns the
Docker topology and the injections.

Limits and their sources:
- RECOVERY_LIMIT: running／ready with the same netmap serial within 10 s of
  the pressure ending (X13／X15 lab: 13 runs, 12 within 10 s, the 13th a
  fixed defect; docs/v2/node/validation.md#x15-livelock).
- RESOLVED_LIMIT_MS: a Mesh name answers through the resolved stub within
  5 s of a resolved restart (`udp/resolved`'s `resolved-restart`,
  docs/v2/install.md: "systemd-resolved 重启后 5 s 内").
- SYNC_RECOVERY_LIMIT: Control's backoff tops at 15 s
  (`meshway-control` netmap/sync.rs BACKOFF_MAX), the Server holds a sync at
  most 54 s (`MAX_SYNC_HOLD`): one retry and one hold after the drop ends.
- EXIT_READY_LIMIT: the exit role is ready and installed again within the
  30 s unverified limit of its owner (X12／X13 `UNVERIFIED_LIMIT`).
- CPU_THREAD_LIMIT／CPU_PROCESS_LIMIT: the busiest meshwayd thread under 5 %
  and the process under 10 % of one core while idle or under a fault other
  than a CPU limit (X18: a level-triggered branch holds a thread at ~100 %;
  release idle 0.56 %; this row runs debug builds with 2 Status reads/s and
  a 5 pps flow).
- FLOW_GAP_LIMIT: 5 pps; at most 2 s between echoes where the flow must
  continue (ten intervals).
- WITHDRAWAL_BOUND: a fault is one withdrawal and one reinstall; ten in one
  stage is a loop (X15: ~37,000 per second).
"""
import json
from datetime import datetime
from pathlib import Path

NODES = ("a", "r", "b", "e")
RECOVERY_LIMIT = 10.0
RESOLVED_LIMIT_MS = 5000
SYNC_RECOVERY_LIMIT = 15.0 + 54.0 + 1.0
EXIT_READY_LIMIT = 30.0
CPU_THREAD_LIMIT = 5.0
CPU_PROCESS_LIMIT = 10.0
FLOW_GAP_LIMIT = 2.0
WITHDRAWAL_BOUND = 10

FAILED = ("Required DNS component failed; suspending packet plane",
          "Required packet plane component failed; suspending packet plane")
REINSTALLED = ("Required DNS component recovered; netmap reinstalled",
               "Required packet plane component recovered; netmap reinstalled")
# Which recovery path a stage took (recorded, not gated).
PATHS = {
    "dnsHealthFailed": "DNS owner health check failed",
    "changeRetry": "Egress capture change failed; read back, retrying",
    "changeCompleted": "Egress capture change completed after retrying",
    "verifyUnavailable": "Egress capture verification unavailable; retrying, nothing new installed meanwhile",
    "ownershipEnded": "Egress capture owner stopping after a failed kernel operation; kernel ownership ends",
    "reproofUnavailable": "Kernel ownership re-proof unavailable; retrying",
    "reproved": "Kernel ownership proved again; the Host owns a new term",
    "reproofConflict": "Kernel ownership re-proof read back a conflict; ownership stays ended until restart",
    "exitAuditUnavailable": "Exit role audit unavailable; retrying, the role and its flows kept",
    "exitAuditVerified": "Exit role audit verified again; ownership kept",
    "exitChangeRetry": "Exit role change failed; read back, retrying",
    "exitCleanupRetry": "Exit role cleanup failed; resources kept, retrying",
    "exitCleanupCompleted": "Exit role cleanup completed after retrying; the role may install again",
}
KNOWN = set(FAILED) | set(REINSTALLED) | set(PATHS.values())


def write(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def samples(path):
    """Status samples of one Node, in time order; unreadable lines skipped."""
    found = []
    if not path.exists():
        return found
    for line in path.read_text(errors="replace").splitlines():
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if isinstance(value, dict) and isinstance(value.get("at"), (int, float)):
            found.append(value)
    return sorted(found, key=lambda value: value["at"])


def serial(sample):
    return ((sample or {}).get("applied") or {}).get("serial")


def is_running(sample, expected):
    return (sample.get("phase") == "running" and sample.get("dataPlane") == "ready"
            and serial(sample) == expected)


def between(values, start, end):
    return [value for value in values if start <= value["at"] <= end and "error" not in value]


def first(values, after, predicate):
    return next((value for value in values if value["at"] >= after and "error" not in value and predicate(value)), None)


def exit_status(family):
    """A family's status: `{"status": "ready"}` (with a reason for others)."""
    value = family.get("status")
    return value.get("status") if isinstance(value, dict) else value


def exit_ready(sample):
    families = (((sample.get("exit") or {}).get("capability") or {}).get("families")) or []
    return bool(families) and all(exit_status(item) == "ready" and item.get("installed") for item in families)


def log_events(path, start, end):
    """(time, message, fields) of the known records in [start, end]."""
    events = []
    if not path.exists():
        return events
    with path.open(errors="replace") as stream:
        for line in stream:
            if not line.startswith("{") or not any(message in line for message in KNOWN):
                continue
            try:
                record = json.loads(line)
                at = datetime.fromisoformat(record["timestamp"].replace("Z", "+00:00")).timestamp()
            except (ValueError, KeyError, TypeError):
                continue
            fields = record.get("fields") or {}
            if start <= at <= end and fields.get("message") in KNOWN:
                events.append((at, fields["message"], fields))
    return events


def withdrawals(events):
    failed = [at for at, message, _ in events if message in FAILED]
    reinstalled = [at for at, message, _ in events if message in REINSTALLED]
    return failed, reinstalled


def paths(events):
    counts = {name: sum(1 for _, message, _ in events if message == text) for name, text in PATHS.items()}
    checks = sorted({fields.get("check") for _, message, fields in events
                     if message == PATHS["ownershipEnded"] and fields.get("check")})
    taken = [name for name in ("changeRetry", "verifyUnavailable", "ownershipEnded", "exitAuditUnavailable")
             if counts[name]]
    return {"counts": {name: count for name, count in counts.items() if count}, "ownershipEndChecks": checks,
            "taken": taken or ["dnsOnly" if counts["dnsHealthFailed"] else "none"]}


def flow(path):
    """A's echo times, in order."""
    echoes = []
    if not path.exists():
        return echoes
    for line in path.read_text(errors="replace").splitlines():
        fields = line.split()
        if len(fields) == 3 and fields[0] == "e" and fields[1].isdigit():
            echoes.append(int(fields[1]) / 1e9)
    return sorted(echoes)


def longest_gap(echoes, start, end):
    inside = [at for at in echoes if start <= at <= end]
    points = [start, *inside, end]
    return round(max(later - earlier for earlier, later in zip(points, points[1:])), 3)


def cpu(start, end):
    """Busiest thread and whole process, percent of one core, between two
    samples of `/proc/<pid>/task/*/stat`; None when the process changed."""
    if not start or not end or start.get("pid") != end.get("pid"):
        return None
    seconds = end["uptime"] - start["uptime"]
    if seconds <= 0:
        return None
    ticks = seconds * end["clkTck"]
    threads = {}
    for tid, thread in end["threads"].items():
        before = start["threads"].get(tid, {}).get("ticks", 0)
        threads[tid] = {"name": thread["name"], "percent": round((thread["ticks"] - before) / ticks * 100, 2)}
    busiest = max(threads.values(), key=lambda thread: thread["percent"], default={"name": None, "percent": 0.0})
    return {"seconds": round(seconds, 1), "processPercent": round((end["process"] - start["process"]) / ticks * 100, 2),
            "busiestThread": busiest, "threads": len(threads)}


def cpu_checks(window):
    """Every Node's measured window within the limits; a missing measurement fails."""
    measured = {role: cpu(*(window or {}).get(role, (None, None))) for role in NODES}
    passed = all(value is not None and value["busiestThread"]["percent"] < CPU_THREAD_LIMIT
                 and value["processPercent"] < CPU_PROCESS_LIMIT for value in measured.values())
    return passed, measured


def recovery(values, after, expected, extra=lambda _: True):
    found = first(values, after, lambda sample: is_running(sample, expected) and extra(sample))
    return None if found is None else round(found["at"] - after, 3)


def never_blocked(by_role, start, end):
    return not any(sample.get("phase") == "blocked" for values in by_role.values() for sample in between(values, start, end))


def judge_stage(stage, by_role, echoes, logs, serials):
    kind, node = stage["kind"], stage["node"]
    injected, restored, end = stage["injectedAt"], stage["restoredAt"], stage["endedAt"]
    values = by_role[node]
    events = log_events(logs[node], injected, end)
    failed, reinstalled = withdrawals(events)
    measured = {"paths": paths(events), "withdrawals": len(failed), "reinstalls": len(reinstalled)}
    checks = {"neverBlocked": never_blocked(by_role, injected, end),
              "withdrawalsBounded": len(failed) <= WITHDRAWAL_BOUND,
              "everyWithdrawalReinstalled": len(failed) == len(reinstalled)}
    idle_passed, measured["idleCpu"] = cpu_checks(stage.get("idleCpu"))
    checks["idleCpu"] = idle_passed
    if stage.get("faultCpu") is not None:
        checks["faultCpu"], measured["faultCpu"] = cpu_checks(stage["faultCpu"])
    if kind == "throttle":
        withdrawn = [sample for sample in between(values, injected, restored + RECOVERY_LIMIT)
                     if sample.get("dataPlane") != "ready"]
        measured["withdrawnSamples"] = len(withdrawn)
        measured["causes"] = sorted({sample.get("lastError") or "null" for sample in withdrawn})
        measured["phases"] = sorted({sample.get("phase") for sample in withdrawn})
        checks["faultObserved"] = bool(failed)
        checks["retryingObserved"] = any(sample.get("phase") == "retrying" for sample in withdrawn)
        checks["causeWhileWithdrawn"] = all(sample.get("lastError") for sample in withdrawn)
        measured["recoveredS"] = recovery(values, restored, serials[node])
        checks["recovered"] = measured["recoveredS"] is not None and measured["recoveredS"] <= RECOVERY_LIMIT
        later = between(values, restored + RECOVERY_LIMIT, end)
        checks["notSuspendedAfter"] = not any(sample.get("dataPlane") == "suspended" for sample in later)
    elif kind == "resolved":
        measured["restarts"] = stage["restarts"]
        recovered = []
        for restart in stage["restarts"]:
            result = restart.get("result") or {}
            recovered.append(recovery(values, restart["requestedAt"], serials[node]))
            restart["running"] = recovered[-1]
        checks["namesAnswered"] = len(stage["restarts"]) == 2 and all(
            (restart.get("result") or {}).get("resolved") is True
            and restart["result"].get("answeredBefore") is True
            and (restart["result"].get("recoveredMs") or RESOLVED_LIMIT_MS + 1) <= RESOLVED_LIMIT_MS
            for restart in stage["restarts"])
        checks["recovered"] = len(recovered) == 2 and all(value is not None and value <= RECOVERY_LIMIT
                                                          for value in recovered)
    elif kind == "outage":
        during = between(values, injected, restored)
        measured["samples"] = len(during)
        measured["lastErrors"] = sorted({sample.get("lastError") or "null" for sample in during})
        measured["dropped"] = stage.get("dropped")
        checks["dropped"] = bool(stage.get("dropped"))
        checks["dataPlaneContinued"] = bool(during) and all(is_running(sample, serials[node]) for sample in during)
        checks["networkObserved"] = any(sample.get("lastError") == "network" and sample.get("syncError") == "network"
                                        for sample in during)
        checks["noNetworkError"] = not any(sample.get("networkError") for sample in between(values, injected, end))
        measured["longestGapS"] = longest_gap(echoes, injected, restored)
        checks["flowContinued"] = measured["longestGapS"] <= FLOW_GAP_LIMIT
        synced = first(values, restored, lambda sample: sample.get("lastError") is None
                       and sample.get("syncError") is None and sample.get("sync") == "connected")
        measured["syncRecoveredS"] = None if synced is None else round(synced["at"] - restored, 3)
        checks["syncRecovered"] = (measured["syncRecoveredS"] is not None
                                   and measured["syncRecoveredS"] <= SYNC_RECOVERY_LIMIT)
    elif kind == "exit":
        measured["recoveredS"] = recovery(values, restored, serials[node])
        checks["recovered"] = measured["recoveredS"] is not None and measured["recoveredS"] <= RECOVERY_LIMIT
        ready = first(values, restored, lambda sample: is_running(sample, serials[node]) and exit_ready(sample))
        measured["exitReadyS"] = None if ready is None else round(ready["at"] - restored, 3)
        checks["exitReady"] = measured["exitReadyS"] is not None and measured["exitReadyS"] <= EXIT_READY_LIMIT
        measured["longestGapS"] = longest_gap(echoes, injected, end)
        checks["unrelatedFlowContinued"] = measured["longestGapS"] <= FLOW_GAP_LIMIT
    if kind in ("throttle", "resolved"):
        after = stage.get("idleCpuStartedAt", end)
        measured["packetRecoveredS"] = next((round(at - restored, 3) for at in echoes if at >= restored), None)
        checks["packetsRecovered"] = (measured["packetRecoveredS"] is not None
                                      and measured["packetRecoveredS"] <= RECOVERY_LIMIT)
        measured["idleLongestGapS"] = longest_gap(echoes, after, end)
        checks["flowAfterRecovery"] = measured["idleLongestGapS"] <= FLOW_GAP_LIMIT
    return {"checks": checks, "measured": measured, "passed": all(checks.values())}


def analyse(report, manifest):
    by_role = {role: samples(report / role / "status.jsonl") for role in NODES}
    logs = {role: report / role / "daemon.stderr" for role in NODES}
    echoes = flow(report / "a" / "probe" / "client.log")
    serials = manifest["serials"]
    results = {"baseline": {}}
    baseline_passed, results["baseline"]["idleCpu"] = cpu_checks(manifest["baseline"]["idleCpu"])
    results["baseline"]["passed"] = baseline_passed
    for name, stage in manifest["stages"].items():
        results[name] = judge_stage(stage, by_role, echoes, logs, serials)
    start, end = manifest["baseline"]["startedAt"], manifest["endedAt"]
    final = {role: next((value for value in reversed(by_role[role]) if "error" not in value), None) for role in NODES}
    gate = {"baseline": baseline_passed,
            **{name: results[name]["passed"] for name in manifest["stages"]},
            "neverBlocked": never_blocked(by_role, start, end),
            "sameNetmapAtEnd": all(serial(final[role]) == serials[role] for role in NODES),
            "stagesRun": list(manifest["stages"]) == list(manifest["plannedStages"])}
    results["gate"] = {"checks": gate, "passed": all(gate.values())}
    return results


if __name__ == "__main__":
    import sys
    directory = Path(sys.argv[1])
    print(json.dumps(analyse(directory, json.loads((directory / "report.json").read_text())), indent=2))
