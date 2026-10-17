import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("exit_evidence", Path(__file__).resolve().parents[1] / "dev/exit-fixture/evidence.py")
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)
spec = importlib.util.spec_from_file_location("exit_ping", Path(__file__).resolve().parents[1] / "dev/exit-fixture/ping.py")
pinging = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pinging)
spec = importlib.util.spec_from_file_location("exit_auto", Path(__file__).resolve().parents[1] / "dev/exit-fixture/auto.py")
auto = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auto)
spec = importlib.util.spec_from_file_location("exit_echo", Path(__file__).resolve().parents[1] / "dev/exit-fixture/echo.py")
echo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(echo)


class ExitEvidenceTests(unittest.TestCase):
    def test_tuple_requires_original_and_reverse_nat_and_owned_mark(self):
        line = "udp 17 29 src=100.64.0.1 dst=11.77.2.3 sport=40000 dport=8080 src=11.77.2.3 dst=11.77.2.2 sport=8080 dport=41000 mark=8 use=1"
        self.assertEqual(evidence.nat_tuple(line, "udp", "100.64.0.1:40000", "11.77.2.3:8080", "11.77.2.2")["reply"]["dport"], "41000")
        self.assertEqual(evidence.nat_tuple("ipv4     2 " + line, "udp", "100.64.0.1:40000", "11.77.2.3:8080", "11.77.2.2")["mark"], 8)
        for invalid in (line.replace("mark=8", "mark=0"), line.replace("dst=11.77.2.2", "dst=100.64.0.1"),
                        line.replace("sport=40000", "sport=40001"), line + "\n" + line):
            with self.assertRaises(RuntimeError):
                evidence.nat_tuple(invalid, "udp", "100.64.0.1:40000", "11.77.2.3:8080", "11.77.2.2")

    def test_component_or_partial_pass_cannot_promote_row(self):
        report = {"stages": {str(i): {"status": "passed"} for i in range(1, 17)},
                  "dnsNoSourceFallback": {"passed": True},
                  "processes": {role: {"ExitCode": 0, "Running": False, "OOMKilled": False}
                                for role in ("controller", "a", "r", "e", "external", "lan", "resolver", "router")}}
        self.assertTrue(evidence.all_stages_passed(report))
        del report["stages"]["12"]
        self.assertFalse(evidence.all_stages_passed(report))
        report["stages"]["12"] = {"status": "passed"}
        report["stages"]["9"]["status"] = "notRun"
        self.assertFalse(evidence.all_stages_passed(report))
        del report["stages"]["11"]
        self.assertFalse(evidence.all_stages_passed(report))
        report["stages"]["11"] = {"status": "passed"}
        report["stages"]["9"]["status"] = "passed"
        report["stages"]["7"]["status"] = "observed"
        self.assertFalse(evidence.all_stages_passed(report))
        report["stages"]["7"]["status"] = "passed"
        report["cleanupErrors"] = ["namespace still running"]
        self.assertFalse(evidence.all_stages_passed(report))

    def test_component_or_partial_pass_cannot_promote_row_without_the_ping_phase(self):
        report = {"stages": {str(i): {"status": "passed"} for i in range(1, 17)},
                  "processes": {}}
        del report["stages"]["13"]
        self.assertFalse(evidence.all_stages_passed(report))


A, R, E = "a-id", "r-id", "e-id"


def record(stream, code=0):
    import json
    return {"exitCode": code, "stdout": "".join(json.dumps(event) + "\n" for event in stream)}


def decided(action, path, rule=None, resolution=None, destination="11.77.0.3"):
    return {"type": "decision", "decision": {"destination": destination, "node": None, "resolution": resolution,
            "profile": "exit", "rule": rule, "action": action, "path": path}}


def reply(source, path):
    return {"type": "echo", "echo": {"sequence": 0, "outcome": "reply", "from": source, "error": None,
            "durationUs": 900, "setup": False, "path": path}}


DONE = {"type": "completed", "sent": 1, "success": 1, "timeout": 0, "failed": 0}


class ExitPingTests(unittest.TestCase):
    def test_exit_ping_needs_the_decision_first_and_replies_on_the_same_path(self):
        good = [decided({"type": "exit", "node": E}, [A, R, E]), reply("11.77.0.3", [A, R, E]), DONE]
        self.assertEqual(pinging.exit_ping(record(good), "11.77.0.3", E, [A, R, E])["success"], 1)
        preparing = {"type": "preparing", "elapsedMs": 900}
        pinging.exit_ping(record([preparing] + good), "11.77.0.3", E, [A, R, E])
        for broken in ([good[1], good[0], DONE],
                       [decided({"type": "direct"}, []), good[1], DONE],
                       [good[0], reply("11.77.0.1", [A, R, E]), DONE],
                       [good[0], reply("11.77.0.3", [A, E]), DONE],
                       [good[0], good[1], dict(DONE, success=0)]):
            with self.assertRaises(RuntimeError):
                pinging.exit_ping(record(broken), "11.77.0.3", E, [A, R, E])
        with self.assertRaises(RuntimeError):
            pinging.exit_ping(record(good, code=1), "11.77.0.3", E, [A, R, E])

    def test_a_name_needs_the_exit_resolver_and_its_association(self):
        name = "svc.name.exit-dns.test"
        resolution = {"name": name, "resolver": {"type": "exit", "node": E}, "addresses": ["11.77.0.3"],
                      "association": {"names": [name], "remainingSeconds": 40}}
        rule = {"pointer": "/routing/egress/profiles/exit/rules/2", "remark": "", "ruleSet": None}
        good = [decided({"type": "exit", "node": E}, [A, R, E], rule, resolution), reply("11.77.0.3", [A, R, E]), DONE]
        pinging.exit_ping(record(good), "11.77.0.3", E, [A, R, E], rule["pointer"], name)
        for change in ({"association": None}, {"resolver": {"type": "system"}}):
            broken = [decided({"type": "exit", "node": E}, [A, R, E], rule, dict(resolution, **change)), good[1], DONE]
            with self.assertRaises(RuntimeError):
                pinging.exit_ping(record(broken), "11.77.0.3", E, [A, R, E], rule["pointer"], name)

    def test_direct_and_denied(self):
        rule = {"pointer": "/routing/egress/profiles/exit/rules/0", "remark": "", "ruleSet": None}
        good = [decided({"type": "direct"}, [], rule, destination="10.9.0.2"), reply("10.9.0.2", []), DONE]
        pinging.local_ping(record(good), "10.9.0.2", "direct", rule["pointer"])
        with self.assertRaises(RuntimeError):
            pinging.local_ping(record([good[0], reply("10.9.0.2", [A, R, E]), DONE]), "10.9.0.2", "direct", rule["pointer"])
        denied = [decided({"type": "denied", "reason": "unauthorized"}, []), {"type": "failed", "code": "permissionDenied"}]
        self.assertEqual(pinging.denied_ping(record(denied, code=1))["reason"], "unauthorized")
        for broken in (record(denied, code=0), record([denied[0], reply("11.77.0.3", []), denied[1]], code=1)):
            with self.assertRaises(RuntimeError):
                pinging.denied_ping(broken)


class ExitAutoTests(unittest.TestCase):
    def test_resolution_is_read_only_while_auto_applies(self):
        state = {"appliedProfile": "auto", "auto": {"profile": "exit", "basis": "label", "region": "US"}}
        self.assertEqual(auto.resolution(state), ("exit", "label", "US"))
        self.assertIsNone(auto.resolution({"appliedProfile": "exit", "auto": None}))

    def test_a_switch_ends_every_flow_and_needs_probes_on_both_sides(self):
        flow = {"tcp": [[1, True], [2, True], [9, False]], "udp": [[1, True], [2, True], [9, False]]}
        self.assertTrue(auto.ended_at_switch(flow, 4, 6))
        # A probe while the switch commits may go either way.
        self.assertTrue(auto.ended_at_switch({**flow, "tcp": [[1, True], [5, True], [9, False]]}, 4, 6))
        self.assertFalse(auto.ended_at_switch({**flow, "udp": [[1, True], [9, True]]}, 4, 6))
        self.assertFalse(auto.ended_at_switch({**flow, "tcp": [[1, False], [9, False]]}, 4, 6))
        self.assertFalse(auto.ended_at_switch({**flow, "tcp": [[1, True]]}, 4, 6))


class ExitEchoTests(unittest.TestCase):
    def test_the_role_outage_is_bounded_by_es_own_timestamps(self):
        import json

        def line(at, message):
            return json.dumps({"timestamp": "2026-10-15T06:23:%06.3fZ" % at, "fields": {"message": message}})
        log = "\n".join([line(50.0, echo.ROLE_STOPPING), line(55.98, echo.ROLE_STOPPING),
                          line(56.566, echo.REPORT_ACKNOWLEDGED), line(57.283, echo.REPORT_ACKNOWLEDGED)])
        base = 1792045380.0  # 2026-10-15T06:23:00Z

        def sample(observed, status="ready", role="r"):
            return {"exit": {"installedRole": role, "capability": {
                "families": [{"family": "ipv4", "status": {"status": status}, "installed": role is not None}],
                "observedAt": "2026-10-15T06:23:%06.3fZ" % observed}}}
        # The sample before the stop still shows the old ready capability;
        # the reinstalled one is observed at 56.625 and acknowledged at 57.283.
        window = [sample(48.0), sample(56.2, "probing"), sample(56.625)]
        begin, end = echo.outage_window(log, window, base + 55.95)
        self.assertAlmostEqual(begin, base + 55.98, places=3)
        self.assertAlmostEqual(end, base + 57.283 + echo.FACT_DELIVERY, places=3)
        with self.assertRaises(RuntimeError):
            echo.outage_window(log, [sample(48.0)], base + 55.95)
        with self.assertRaises(RuntimeError):
            echo.outage_window(log, window, base + 58.0)
        # Only a ready capability of an installed role after the stop ends it:
        # a probing or unavailable one, a missing capability or role does not.
        stopped = [sample(48.0), sample(56.2, "probing")]
        for after in (sample(56.625, "probing"),
                      {"exit": {"installedRole": "r", "capability": {
                          "families": [{"family": "ipv4", "installed": True,
                                        "status": {"status": "unavailable", "reason": "ownershipConflict"}}],
                          "observedAt": "2026-10-15T06:23:56.625Z"}}},
                      {"exit": {"installedRole": "r", "capability": None}},
                      {"exit": None},
                      sample(56.625, role=None)):
            with self.assertRaises(RuntimeError):
                echo.outage_window(log, stopped + [after], base + 55.95)


if __name__ == "__main__":
    unittest.main()
