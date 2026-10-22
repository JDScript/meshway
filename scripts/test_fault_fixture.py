"""Offline gates of the fault/throttle row: synthetic reports, no Docker."""
import copy
import importlib.util
import json
from datetime import datetime, timezone
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


evidence = load("fault_evidence", "dev/fault-fixture/evidence.py")
runtime = load("fault_runtime", "dev/fault-fixture/runtime.py")

SERIAL = 3
NODES = evidence.NODES


def cpu_sample(uptime, process, threads, pid=7):
    return {"pid": pid, "clkTck": 100, "uptime": uptime, "process": process,
            "threads": {tid: {"name": "tokio-rt-worker", "ticks": ticks} for tid, ticks in threads.items()}}


def quiet(start, seconds=60.0):
    """Every Node: 1 % process, 0.5 % busiest thread over the window."""
    return {role: (cpu_sample(start, 100, {"1": 50, "2": 10}),
                   cpu_sample(start + seconds, 100 + seconds, {"1": 50 + seconds / 2, "2": 10}))
            for role in NODES}


def running(at, **change):
    return {"at": at, "phase": "running", "dataPlane": "ready", "lastError": None, "syncError": None,
            "sync": "connected", "applied": {"serial": SERIAL},
            "exit": {"capability": {"families": [{"family": "ipv4", "status": {"status": "ready"}, "installed": True}]}},
            **change}


def log(at, message, **fields):
    stamp = datetime.fromtimestamp(at, timezone.utc).isoformat().replace("+00:00", "Z")
    return json.dumps({"timestamp": stamp, "level": "WARN", "fields": {"message": message, **fields}}) + "\n"


class Report:
    """A passing row: baseline 1000–1070, then the five stages."""

    def __init__(self, long_fault=True):
        self.status = {role: {} for role in NODES}
        self.windows = {role: [] for role in NODES}
        self.logs = {role: [] for role in NODES}
        self.manifest = {"serials": {role: SERIAL for role in NODES}, "plannedStages": list(runtime.STAGES),
                         "baseline": {"startedAt": 1000.0, "idleCpu": quiet(1000.0)}, "stages": {}}
        stages = self.manifest["stages"]
        stages["cpu-long"] = self.throttle(1100.0, 90, long_fault)
        stages["cpu-short"] = self.throttle(1300.0, 30)
        stages["resolved-restart"] = {
            "kind": "resolved", "node": "b", "injectedAt": 1450.0, "restoredAt": 1462.0,
            "restarts": [{"requestedAt": 1450.0, "result": {"resolved": True, "answeredBefore": True, "recoveredMs": 600}},
                         {"requestedAt": 1462.0, "result": {"resolved": True, "answeredBefore": True, "recoveredMs": 700}}],
            "faultCpu": quiet(1450.0, 24.0), "idleCpuStartedAt": 1480.0, "idleCpu": quiet(1480.0), "endedAt": 1540.0}
        for at in (1450.0, 1462.0):
            self.withdraw("b", at + 0.5, at + 2.0)
        stages["control-outage"] = {
            "kind": "outage", "node": "b", "injectedAt": 1600.0, "restoredAt": 1700.0,
            "dropped": {"input": 12, "output": 3}, "faultCpu": quiet(1610.0, 80.0),
            "idleCpuStartedAt": 1730.0, "idleCpu": quiet(1730.0), "endedAt": 1790.0}
        for index in range(160):
            at = 1640.0 + index / 2
            self.status["b"][at] = running(at, lastError="network", syncError="network", sync="reconnecting")
        stages["exit-audit"] = {"kind": "exit", "node": "e", "injectedAt": 1800.0, "restoredAt": 1812.0,
                                "idleCpuStartedAt": 1820.0, "idleCpu": quiet(1820.0), "endedAt": 1880.0}
        self.logs["e"].append(log(1803.0, evidence.PATHS["exitAuditUnavailable"]))
        self.logs["e"].append(log(1813.0, evidence.PATHS["exitAuditVerified"]))
        self.manifest["endedAt"] = 1880.0
        self.echoes = [at / 5 for at in range(5000, 9400)]

    def throttle(self, injected, seconds, fault=True):
        restored = injected + seconds
        if fault:
            self.withdraw("b", injected + 3.0, restored + 1.5, cause="dns", phase="retrying")
            self.logs["b"].append(log(injected + 2.9, evidence.PATHS["changeRetry"], operation="pause"))
        return {"kind": "throttle", "node": "b", "seconds": seconds, "injectedAt": injected, "restoredAt": restored,
                "idleCpuStartedAt": restored + 7, "idleCpu": quiet(restored + 7), "endedAt": restored + 67}

    def withdraw(self, role, start, end, cause="dns", phase="retrying"):
        self.logs[role].append(log(start, evidence.FAILED[0]))
        self.logs[role].append(log(end, evidence.REINSTALLED[0], trigger="offer"))
        self.windows[role].append((start, end))
        at = start
        while at < end:
            self.status[role][at] = {"at": at, "phase": phase, "dataPlane": "failed", "lastError": cause,
                                     "applied": None}
            at += 2.0

    def write(self, directory):
        for role in NODES:
            (directory / role).mkdir()
            samples = {at / 2: running(at / 2) for at in range(1990, 3780)
                       if not any(start <= at / 2 <= end for start, end in self.windows[role])}
            samples.update(self.status[role])
            with (directory / role / "status.jsonl").open("w") as stream:
                for at in sorted(samples):
                    stream.write(json.dumps(samples[at]) + "\n")
            (directory / role / "daemon.stderr").write_text("".join(self.logs[role]) + "not json\n")
        (directory / "a" / "probe").mkdir()
        (directory / "a" / "probe" / "client.log").write_text(
            "".join(f"s {int(at * 1e9)} 1\ne {int(at * 1e9)} 1\n" for at in self.echoes))
        return evidence.analyse(directory, self.manifest)


def analyse(report):
    with tempfile.TemporaryDirectory() as directory:
        return report.write(Path(directory))


class GateTests(unittest.TestCase):
    def test_a_recovering_row_passes_and_records_its_path(self):
        results = analyse(Report())
        self.assertTrue(results["gate"]["passed"], json.dumps(results["gate"]))
        self.assertEqual(results["cpu-long"]["measured"]["paths"]["taken"], ["changeRetry"])
        self.assertEqual(results["cpu-long"]["measured"]["causes"], ["dns"])
        self.assertEqual(results["exit-audit"]["measured"]["paths"]["taken"], ["exitAuditUnavailable"])
        self.assertLessEqual(results["control-outage"]["measured"]["syncRecoveredS"], evidence.SYNC_RECOVERY_LIMIT)

    def failed(self, report, stage, check):
        results = analyse(report)
        self.assertFalse(results["gate"]["passed"])
        self.assertFalse(results[stage]["checks"][check] if stage != "gate" else results["gate"]["checks"][check],
                         json.dumps(results[stage].get("checks", results[stage])))

    def test_a_spinning_thread_fails_the_idle_window(self):
        # X18: one tokio worker at ~100 % while the Node is idle.
        report = Report()
        start = report.manifest["stages"]["cpu-short"]["idleCpuStartedAt"]
        report.manifest["stages"]["cpu-short"]["idleCpu"]["r"] = (
            cpu_sample(start, 0, {"1": 0}), cpu_sample(start + 60, 6000, {"1": 5990}))
        self.failed(report, "cpu-short", "idleCpu")
        report = Report()
        report.manifest["baseline"]["idleCpu"]["a"] = (cpu_sample(1000, 0, {"1": 0}), cpu_sample(1060, 6000, {"1": 5990}))
        self.failed(report, "gate", "baseline")

    def test_a_spin_during_the_outage_fails_its_fault_window(self):
        report = Report()
        report.manifest["stages"]["control-outage"]["faultCpu"]["b"] = (
            cpu_sample(1610, 0, {"1": 0, "2": 0}), cpu_sample(1690, 900, {"1": 880, "2": 20}))
        self.failed(report, "control-outage", "faultCpu")

    def test_a_missing_or_restarted_cpu_sample_is_no_pass(self):
        report = Report()
        report.manifest["stages"]["cpu-long"]["idleCpu"]["e"] = (None, None)
        self.failed(report, "cpu-long", "idleCpu")
        report = Report()
        start, end = report.manifest["stages"]["exit-audit"]["idleCpu"]["e"]
        report.manifest["stages"]["exit-audit"]["idleCpu"]["e"] = (start, dict(end, pid=99))
        self.failed(report, "exit-audit", "idleCpu")

    def test_a_node_that_does_not_heal_fails_recovery(self):
        # X13: transiently ended ownership never proved again.
        report = Report()
        report.withdraw("b", 1190.0, 1300.0)
        self.failed(report, "cpu-long", "recovered")

    def test_blocked_and_a_null_cause_while_withdrawn_fail(self):
        report = Report()
        report.withdraw("b", 1104.0, 1192.0, phase="blocked")
        results = analyse(report)
        self.assertFalse(results["cpu-long"]["checks"]["neverBlocked"])
        self.assertFalse(results["gate"]["checks"]["neverBlocked"])
        report = Report()
        report.withdraw("b", 1304.0, 1331.0, cause=None)
        self.failed(report, "cpu-short", "causeWhileWithdrawn")

    def test_a_withdrawal_loop_and_a_missing_reinstall_fail(self):
        # X15: the failure and recovery branches alternate.
        report = Report()
        for index in range(40):
            report.logs["b"].append(log(1335.0 + index / 100, evidence.FAILED[0]))
            report.logs["b"].append(log(1335.0 + index / 100, evidence.REINSTALLED[0]))
        self.failed(report, "cpu-short", "withdrawalsBounded")
        report = Report()
        report.logs["b"].append(log(1340.0, evidence.FAILED[0]))
        self.failed(report, "cpu-short", "everyWithdrawalReinstalled")

    def test_a_throttle_without_a_fault_proves_nothing(self):
        self.failed(Report(long_fault=False), "cpu-long", "faultObserved")

    def test_a_stuck_suspension_and_a_lost_flow_fail(self):
        report = Report()
        for index in range(20):
            at = 1250.0 + index
            report.status["b"][at] = running(at, dataPlane="suspended", phase="synchronizing", applied=None,
                                             lastError="dns")
        self.failed(report, "cpu-long", "notSuspendedAfter")
        report = Report()
        report.echoes = [at for at in report.echoes if not 1200 <= at <= 1210]
        self.failed(report, "cpu-long", "flowAfterRecovery")

    def test_the_outage_needs_drops_a_network_cause_a_running_plane_and_a_sync_again(self):
        report = Report()
        report.manifest["stages"]["control-outage"]["dropped"] = None
        self.failed(report, "control-outage", "dropped")
        report = Report()
        report.status["b"] = {at: value for at, value in report.status["b"].items() if not 1600 <= at <= 1720}
        self.failed(report, "control-outage", "networkObserved")
        report = Report()
        report.status["b"][1650.25] = running(1650.25, networkError={"problem": "dns"}, lastError="network")
        self.failed(report, "control-outage", "noNetworkError")
        report = Report()
        report.status["b"][1650.25] = running(1650.25, dataPlane="suspended", applied=None)
        self.failed(report, "control-outage", "dataPlaneContinued")
        report = Report()
        for index in range(200):
            at = 1700.0 + index / 2
            report.status["b"][at] = running(at, lastError="network", syncError="network", sync="reconnecting")
        self.failed(report, "control-outage", "syncRecovered")
        report = Report()
        report.echoes = [at for at in report.echoes if not 1650 <= at <= 1655]
        self.failed(report, "control-outage", "flowContinued")

    def test_resolved_restarts_must_answer_within_five_seconds(self):
        report = Report()
        report.manifest["stages"]["resolved-restart"]["restarts"][1]["result"] = {
            "resolved": True, "answeredBefore": True, "recoveredMs": 5200}
        self.failed(report, "resolved-restart", "namesAnswered")
        # A name that did not answer before the restart proves nothing.
        report = Report()
        report.manifest["stages"]["resolved-restart"]["restarts"][0]["result"]["answeredBefore"] = False
        self.failed(report, "resolved-restart", "namesAnswered")
        report = Report()
        report.manifest["stages"]["resolved-restart"]["restarts"][0]["result"] = None
        self.failed(report, "resolved-restart", "namesAnswered")

    def test_the_exit_role_must_be_ready_again(self):
        report = Report()
        for index in range(80):
            at = 1812.0 + index / 2
            report.status["e"][at] = running(at, exit={"capability": {"families": [
                {"family": "ipv4", "status": {"status": "probing"}, "installed": False}]}})
        self.failed(report, "exit-audit", "exitReady")

    def test_a_changed_netmap_and_a_skipped_stage_fail_the_row(self):
        report = Report()
        report.status["a"][1895.0] = running(1895.0, applied={"serial": SERIAL + 1})
        report.manifest["endedAt"] = 1900.0
        self.failed(report, "gate", "sameNetmapAtEnd")
        report = Report()
        del report.manifest["stages"]["exit-audit"]
        self.failed(report, "gate", "stagesRun")


class SampleTests(unittest.TestCase):
    def test_proc_stat_with_spaces_and_parentheses_in_the_name(self):
        line = "42 (tokio (rt) worker) S 1 42 42 0 -1 4194560 10 0 0 0 250 75 0 0 20 0 9 0 100 0 0"
        self.assertEqual(runtime.parse_stat(line), ("tokio (rt) worker", 325))
        sample = runtime.parse_cpu("pid 42\nclk 100\nuptime 12.5\nproc " + line + "\ntask 43 " + line + "\n")
        self.assertEqual((sample["pid"], sample["process"], sample["threads"]["43"]["ticks"]), (42, 325, 325))
        with self.assertRaises(RuntimeError):
            runtime.parse_cpu("pid 42\n")

    def test_a_thread_born_inside_the_window_counts_from_zero(self):
        start = cpu_sample(10, 0, {"1": 0})
        end = cpu_sample(20, 60, {"1": 10, "2": 50})
        measured = evidence.cpu(start, end)
        self.assertEqual((measured["processPercent"], measured["busiestThread"]["percent"]), (6.0, 5.0))
        self.assertIsNone(evidence.cpu(start, copy.deepcopy(start)))

    def test_gaps_count_the_window_edges(self):
        self.assertEqual(evidence.longest_gap([1.0, 2.0], 0.0, 5.0), 3.0)
        self.assertEqual(evidence.longest_gap([], 0.0, 5.0), 5.0)


if __name__ == "__main__":
    unittest.main()
