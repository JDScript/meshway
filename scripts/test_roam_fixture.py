import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("roam_evidence", Path(__file__).resolve().parents[1] / "dev/roam-fixture/evidence.py")
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)

ROUTED = 1000.0


def ns(at):
    return str(int(at * 1e9))


class TraversalGateTests(unittest.TestCase):
    def test_the_moved_a_prepares_first_and_b_accepts_beside_its_old_half(self):
        good = {"reacquiredS": 3.1, "firstPreparedBy": "a", "bAcceptedBesideChannel": True}
        self.assertTrue(all(evidence.traversal_checks(good).values()))
        for change in ({"reacquiredS": None}, {"firstPreparedBy": "b"}, {"firstPreparedBy": None},
                       {"bAcceptedBesideChannel": False}):
            self.assertFalse(all(evidence.traversal_checks(dict(good, **change)).values()), change)


class RelayGateTests(unittest.TestCase):
    def test_new_flows_of_the_gap_need_flows_replies_and_relay_traffic(self):
        self.assertTrue(evidence.through_relay({1: 0.0, 2: 0.25}, {1, 2}, 12.0))
        # An empty window has no sample: not applicable, not a pass.
        self.assertIsNone(evidence.through_relay({}, set(), 12.0))
        # A missing reply, no or zero relay traffic: no evidence.
        self.assertFalse(evidence.through_relay({1: 0.0, 2: 0.25}, {1}, 12.0))
        self.assertFalse(evidence.through_relay({1: 0.0}, {1}, None))
        self.assertFalse(evidence.through_relay({1: 0.0}, {1}, 0.0))

    def cell(self, report, flows, counters):
        (report / "a" / "traffic-traversal-flows").mkdir(parents=True)
        (report / "a" / "traffic-traversal-flows" / "client.log").write_text("".join(flows))
        (report / "r").mkdir()
        (report / "r" / "r-rx.log").write_text("".join(f"{ns(at)} {value}\n" for at, value in counters))
        cell = {"routedAt": ROUTED, "directLeftAt": ROUTED + 0.5, "recoveredAt": ROUTED + 3.0, "cell": "soft",
                "relayStreamAt": ROUTED, "relayStreamStoppedAt": ROUTED + 5.0}
        measured = {"reacquiredS": 2.5, "udpLongestGapS": 3.0}
        return evidence.relay_checks(report, cell, measured, "traversal"), measured["relay"]

    def test_the_relay_cell_rejects_an_empty_gap_and_an_idle_relay(self):
        answered = [f"fsend {ns(ROUTED + 1.0)} 1\n", f"fecho {ns(ROUTED + 1.2)} 1\n"]
        busy = [(ROUTED - 3, 100), (ROUTED - 1, 400)]
        with tempfile.TemporaryDirectory() as directory:
            checks, relay = self.cell(Path(directory), answered, busy)
            self.assertTrue(checks["newFlowsThroughR"])
            self.assertEqual((relay["newFlowsInGap"], relay["newFlowsAnsweredThroughR"]), (1, 1))
        for flows, counters in (([], busy), (answered, [(ROUTED - 3, 100), (ROUTED - 1, 100)]),
                                (answered[:1], busy)):
            with tempfile.TemporaryDirectory() as directory:
                checks, _ = self.cell(Path(directory), flows, counters)
                self.assertFalse(checks["newFlowsThroughR"])

    def test_a_flow_that_could_not_be_sent_in_the_gap_is_an_unanswered_one(self):
        busy = [(ROUTED - 3, 100), (ROUTED - 1, 400)]
        flows = [f"fsend {ns(ROUTED + 1.0)} 1 20001\n", f"fecho {ns(ROUTED + 1.2)} 1\n",
                 f"ferror {ns(ROUTED + 1.25)} 2 98\n"]
        with tempfile.TemporaryDirectory() as directory:
            checks, relay = self.cell(Path(directory), flows, busy)
            self.assertFalse(checks["newFlowsThroughR"])
            self.assertEqual((relay["newFlowsInGap"], relay["newFlowsAnsweredThroughR"]), (2, 1))

    def test_a_gap_flow_on_an_earlier_flows_source_port_is_counted_as_a_reused_tuple(self):
        busy = [(ROUTED - 3, 100), (ROUTED - 1, 400)]
        flows = [f"fsend {ns(ROUTED - 20)} 1 41420\n", f"fecho {ns(ROUTED - 20)} 1\n",
                 f"fsend {ns(ROUTED + 1.0)} 2 41420\n", f"fsend {ns(ROUTED + 1.25)} 3 41421\n",
                 f"fecho {ns(ROUTED + 1.3)} 3\n"]
        with tempfile.TemporaryDirectory() as directory:
            checks, relay = self.cell(Path(directory), flows, busy)
            # Still a gap flow that must be answered: the gate is unchanged.
            self.assertFalse(checks["newFlowsThroughR"])
            self.assertEqual(relay["newFlowsReusingTuple"], 1)


class FlowPortTests(unittest.TestCase):
    def test_no_flow_port_repeats_within_the_flow_idle_timeout_nor_is_ephemeral(self):
        spec = importlib.util.spec_from_file_location(
            "roam_traffic", Path(__file__).resolve().parents[1] / "dev/roam-fixture/traffic.py")
        traffic = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(traffic)
        window = int(evidence.FLOW_IDLE_S / 0.25) + 1
        for start in (0, traffic.FLOW_PORTS - window // 2):
            ports = [traffic.flow_port(identifier) for identifier in range(start, start + window)]
            self.assertEqual(len(set(ports)), window)
        self.assertLess(traffic.flow_port(traffic.FLOW_PORTS - 1), 32768, "below Linux's ephemeral range")


if __name__ == "__main__":
    unittest.main()
