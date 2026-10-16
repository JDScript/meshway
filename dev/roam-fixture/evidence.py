"""Evidence of the roaming rows: parses the actors' traffic logs, R's
counters and the daemons' JSON logs, and judges every cell's gate. Pure
functions of the report directory and the manifest; `runtime.py` owns the
Docker topology and the perturbations."""
import json
from datetime import datetime

SCENARIOS = {
    "direct": {"phases": ("dialer", "listener"), "cells": ("soft", "hard", "dual-stack"), "nodes": ("a", "b", "c")},
    "nat": {"phases": ("traversal",), "cells": ("soft", "hard"), "nodes": ("a", "b", "c")},
    "relay": {"phases": ("traversal",), "cells": ("soft", "hard"), "nodes": ("a", "b", "c", "r")},
    "flap": {"phases": ("dialer",), "cells": ("soft", "burst", "hard"), "nodes": ("a", "b", "c")},
    "exit": {"phases": ("dialer",), "cells": ("soft", "hard"), "nodes": ("a", "b", "c")},
}
# D14: with the peer directly reachable, the first inner reply within 3 s of
# the new uplink's route, on the same circuit; behind two NATs (a new
# coordinated traversal), within 5 s.
FIRST_REPLY_LIMIT = {"direct": 3.0, "flap": 3.0, "nat": 5.0, "relay": 5.0, "exit": 3.0}
# exit: the role reports `probing` after A's path change and `ready` once
# probed again (1 s later, connectivity.md); within this of the new route.
EXIT_READY_LIMIT = 5.0
# Each path change publishes candidates through State reports spaced 100 ms
# for 1.5 s (connectivity.md, roaming): a cell of n path changes accepts at
# most n × 16 reports with candidates, plus one for its final mapping.
REPORTS_PER_PATH_CHANGE = 16


def write(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def parse_client(path):
    events = {"usend": {}, "uecho": [], "techo": [], "pecho": [], "terror": [], "tclosed": [], "tconnect": [],
              "fsend": {}, "fecho": {}}
    if not path.exists():
        return events
    for line in path.read_text().splitlines():
        fields = line.split()
        if len(fields) < 2:
            continue
        kind, at = fields[0], int(fields[1]) / 1e9
        if kind == "usend":
            events["usend"][int(fields[2])] = at
        elif kind == "uecho":
            events["uecho"].append((at, int(fields[2])))
        elif kind in ("fsend", "fecho") and fields[2].isdigit():
            events[kind].setdefault(int(fields[2]), at)
        elif kind in events and isinstance(events[kind], list):
            events[kind].append((at, *fields[2:]))
    return events


def parse_received(path):
    received = {}
    for line in path.read_text().splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[1].isdigit():
            received.setdefault(int(fields[1]), int(fields[0]) / 1e9)
    return received


EVENT_MESSAGES = ("Dataplane circuit retired", "Connectivity channel transition", "Connectivity roam",
                  "Underlay network change classified", "Known peer facts updated",
                  "State candidate publication acknowledged", "Traversal attempt active",
                  "Traversal prepared", "Traversal attempt ended without a channel",
                  "Traversal accepted beside a selected channel", "Traversal reservation refused",
                  "Connectivity channel health ended")
# The exit role's own records (the `exit` row): every one is kept.
EXIT_PREFIX = "Exit"
# Of those, the role ending or refusing flows.
# A flow refused while A has no route at all (between hard's disconnect and
# its new default route) is the kernel's answer, recorded but no failure.
EXIT_FAILURES = ("Exit-owned kernel object", "Exit owner stopping", "Exit role failed",
                 "Exit role installation failed", "Exit role change failed")


def daemon_events(path, start, end):
    """Daemon JSON log records of interest in [start, end] (Unix seconds)."""
    records = []
    for line in path.read_text(errors="replace").splitlines():
        if not line.startswith("{"):
            continue
        try:
            record = json.loads(line)
        except ValueError:
            continue
        stamp = record.get("timestamp")
        if not stamp:
            continue
        at = datetime.fromisoformat(stamp.replace("Z", "+00:00")).timestamp()
        if not start <= at <= end:
            continue
        fields = record.get("fields", {})
        message = fields.get("message", "")
        if (message in EVENT_MESSAGES or message.startswith(EXIT_PREFIX)
                or fields.get("operation") in ("roam", "probe_recovery")):
            if "install_json" in fields:
                try:
                    fields = {**fields, "install": json.loads(fields["install_json"])}
                except ValueError:
                    pass
                fields.pop("install_json", None)
            records.append({"at": at, **fields})
    return records


def installs_of(events, peer):
    """Channel installations toward `peer`: (time, channel ID, remote, acquisition)."""
    found = []
    for event in events:
        installed = event.get("install") or {}
        channel = installed.get("channel") or {}
        if event.get("operation") == "install" and channel.get("peerNodeId") == peer:
            found.append((event["at"], channel.get("linkId"), channel.get("remoteAddress"), channel.get("acquisition")))
    return found


def longest_gap(start, end, arrivals):
    # The window's edges count: no arrival at all is one stall to its end.
    times = [start, *sorted(at for at in arrivals if start <= at <= end), end]
    return max(b - a for a, b in zip(times, times[1:]))


def measure(cell, client, received, t_route, end):
    """First replies after the new route, loss and the TCP stall of one cell."""
    def first(arrivals):
        later = [at for at, *_ in arrivals if t_route <= at <= end]
        return round(min(later) - t_route, 3) if later else None

    sent = {seq: at for seq, at in client["usend"].items() if cell["perturbedAt"] <= at <= end - 2}
    echoed = {seq for at, seq in client["uecho"] if seq in sent}
    at_b = {seq for seq in sent if seq in received}
    echo_after = [(at, seq) for at, seq in client["uecho"] if at >= t_route and client["usend"].get(seq, 0) >= t_route]
    return {
        "firstUdpReplyS": round(min(at for at, _ in echo_after) - t_route, 3) if echo_after else None,
        "firstPingReplyS": first(client["pecho"]),
        "firstTcpEchoS": first(client["techo"]),
        "udpSent": len(sent),
        "udpLostToB": len(sent) - len(at_b),
        "udpLostFromB": len(at_b) - len(echoed),
        "udpLongestGapS": round(longest_gap(cell["perturbedAt"], end, [at for at, _ in client["uecho"]]), 3),
        "tcpLongestGapS": round(longest_gap(cell["perturbedAt"], end, [at for at, *_ in client["techo"]]), 3),
        "tcpErrors": [event for event in client["terror"] + client["tclosed"] if event[0] <= end],
    }


def rate(samples, start, end):
    """Received packets per second at R between two times, from its counter samples."""
    inside = [(at, value) for at, value in samples if start <= at <= end]
    if len(inside) < 2:
        return None
    (first_at, first), (last_at, last) = inside[0], inside[-1]
    return round((last - first) / (last_at - first_at), 1) if last_at > first_at else None


def analyse(report, manifest):
    scenario = manifest.get("scenario", "direct")
    limit = FIRST_REPLY_LIMIT[scenario]
    results = {"cells": {}}
    failures = []
    nodes = manifest["nodes"]
    # The measured Mesh flow: A to B, or in `exit` B to its exit A.
    sender, receiver = ("b", "a") if scenario == "exit" else ("a", "b")
    for name, cell in manifest["cells"].items():
        phase = name.split("/")[0]
        client = parse_client(report / sender / f"traffic-{phase}-{receiver}" / "client.log")
        received = parse_received(report / receiver / f"traffic-{phase}" / "udp-received.log")
        lan = parse_client(report / "a" / f"traffic-{phase}-c" / "client.log")
        end = cell["observedUntil"]
        measured = measure(cell, client, received, cell["routedAt"], end)
        lan_measured = measure(cell, lan, parse_received(report / "c" / f"traffic-{phase}" / "udp-received.log"),
                               cell["perturbedAt"], end)
        before, after = cell["before"], cell["after"]
        a_paths_kept = sorted(set(before["a"]["paths"]) & set(after["a"]["paths"]))
        events = daemon_events(report / "a" / "daemon.stderr", cell["perturbedAt"], end)
        b_events = daemon_events(report / "b" / "daemon.stderr", cell["perturbedAt"], end)
        retired = [event for event in events if event.get("message") == "Dataplane circuit retired"]
        # STUN → candidate publication: A's first accepted State report that
        # carries a candidate again after the path change withdrew them; the
        # peer's receipt: B's first peer-fact update after it.
        published = None
        withdrawn = False
        for event in events:
            if event.get("message") != "State candidate publication acknowledged" or event["at"] < cell["routedAt"]:
                continue
            if event.get("candidate_count") == 0:
                withdrawn = True
            elif withdrawn or published is None:
                published = event["at"]
                if withdrawn:
                    break
        received_at = next((event["at"] for event in b_events if event.get("message") == "Known peer facts updated"
                            and published is not None and event["at"] >= published), None)
        # Re-acquisition: A's first channel installed toward B after the new route.
        installed = [item for item in installs_of(events, nodes["b"]) if item[0] >= cell["routedAt"]]
        measured.update({
            "circuitsBefore": before["a"]["paths"], "circuitsAfter": after["a"]["paths"],
            "sameCircuit": bool(a_paths_kept) and before["a"]["paths"] == after["a"]["paths"],
            "circuitsRetired": len(retired),
            "aChannelToB": {"before": before["a"]["links"].get(nodes["b"]), "after": after["a"]["links"].get(nodes["b"])},
            "bChannelToA": {"before": before["b"]["links"].get(nodes["a"]), "after": after["b"]["links"].get(nodes["a"])},
            "aEndpointOfB": {"before": before["aStatus"]["b"], "after": after["aStatus"]["b"]},
            "bEndpointOfA": {"before": before["bStatus"], "after": after["bStatus"]},
            "aChannelToC": {"before": before["a"]["links"].get(nodes["c"]), "after": after["a"]["links"].get(nodes["c"])},
            "candidatePublishedS": round(published - cell["routedAt"], 3) if published else None,
            "peerReceivedCandidateS": round(received_at - published, 3) if published and received_at else None,
            "reacquiredS": round(installed[0][0] - cell["routedAt"], 3) if installed else None,
            "reacquiredBy": installed[0][3] if installed else None,
            "lanUdpLost": lan_measured["udpLostToB"] + lan_measured["udpLostFromB"],
            "lanUdpLongestGapS": lan_measured["udpLongestGapS"],
            "aSockets": {"before": before["a"]["sockets"], "after": after["a"]["sockets"]},
            "aRoamEvents": [event for event in events if event.get("message") not in
                            ("Dataplane circuit retired", "State candidate publication acknowledged")][:96],
            "bRoamEvents": b_events[:96],
        })
        measured["unrelatedChannelUnchanged"] = (measured["aChannelToC"]["before"] == measured["aChannelToC"]["after"]
                                                 and measured["aChannelToC"]["before"] is not None)
        # A's own reaction: the classified path changes and Manager reactions.
        classified = [event for event in events if event.get("message") == "Underlay network change classified"
                      and event["at"] >= cell["routedAt"] - 0.05]
        measured["aClassified"] = [{"atS": round(event["at"] - cell["routedAt"], 3), "class": event.get("class")}
                                   for event in classified][:16]
        measured["aRoamReactions"] = sum(1 for event in events if event.get("message") == "Connectivity roam"
                                         and event.get("cause") == "path_changed" and event["at"] >= cell["routedAt"])
        if scenario == "exit":
            # B's exit flows start and end per cell; B's circuits from before
            # remain.
            measured["sameCircuit"] = bool(before["b"]["paths"]) and set(before["b"]["paths"]) <= set(after["b"]["paths"])
        if scenario == "relay":
            # Flows started during the gap add a circuit through R; every
            # circuit from before must remain.
            measured["sameCircuit"] = bool(before["a"]["paths"]) and set(before["a"]["paths"]) <= set(after["a"]["paths"])
            measured["relayCircuits"] = [key for key, nodes in after["a"]["pathNodes"].items() if len(nodes or []) > 2]
        first = measured["firstUdpReplyS"]
        checks = {"firstReply": first is not None and first <= limit, "sameCircuit": measured["sameCircuit"],
                  "tcpAlive": not measured["tcpErrors"], "unrelatedChannel": measured["unrelatedChannelUnchanged"]}
        if scenario in ("nat", "relay"):
            # Who started the new traversal: the roaming A's first prepare
            # after the route, before any of B's.
            prepared = [(event["at"], "a") for event in events if event.get("message") == "Traversal prepared"] + [
                (event["at"], "b") for event in b_events if event.get("message") == "Traversal prepared"]
            prepared = sorted(item for item in prepared if item[0] >= cell["routedAt"])
            measured["firstPreparedBy"] = prepared[0][1] if prepared else None
            measured["bAcceptedBesideChannel"] = any(
                event.get("message") == "Traversal accepted beside a selected channel" for event in b_events)
            checks["reacquired"] = measured["reacquiredS"] is not None
        if scenario == "relay":
            checks.update(relay_checks(report, cell, measured, phase))
        if scenario == "flap":
            measured["cell"] = name.split("/")[1]
            checks.update(flap_checks(cell, measured, events, b_events, manifest))
        if name.endswith("/dual-stack"):
            # Only A's IPv4 route toward its peers moved (its IPv6 Server
            # path did not): a path change A itself reacts to.
            first_path = next((item["atS"] for item in measured["aClassified"]
                               if item["class"] in ("Path", "path")), None)
            measured["aPathClassifiedS"] = first_path
            measured["aServerRoute"] = cell.get("aServerRoute")
            checks["pathClassified"] = first_path is not None and first_path <= 1.0
            checks["roamReaction"] = measured["aRoamReactions"] >= 1
        if scenario == "exit":
            checks.update(exit_checks(report, cell, measured, phase, name.split("/")[1], events, manifest))
        measured["checks"] = checks
        passed = all(checks.values())
        measured["gatePassed"] = passed
        if not passed:
            failures.append(name)
        results["cells"][name] = measured
    expected = len(SCENARIOS[scenario]["phases"]) * len(SCENARIOS[scenario]["cells"])
    results["gate"] = {"passed": not failures and len(results["cells"]) == expected, "failedCells": failures,
                       "rule": f"first inner UDP reply ≤ {limit} s after the new route, same circuit, "
                               "TCP stream not reset, unrelated LAN channel unchanged" + {
                                   "nat": "; a new channel to B", "direct": "",
                                   "relay": "; a new channel to B; the old direct flow's gap ≤ re-acquisition + 1 s; "
                                            "new flows in the gap answered through R; a flow started on R stays on R "
                                            "and a flow started after re-acquisition does not cross R",
                                   "flap": "; one path reaction per classified path change; candidate reports bounded; "
                                           "no circuit retired; B follows A's final source",
                                   "exit": "; A's exit capability probing then ready again; the cell's exit TCP "
                                           "connection to T ends; new TCP and UDP flows to T answered from A's new "
                                           "uplink; A's exit role kept"}[scenario]
                       + ("; dual-stack: A classifies a path change within 1 s and reacts" if scenario == "direct" else "")}
    write(report / "roam-results.json", results)
    return results


def exit_checks(report, cell, measured, phase, name, events, manifest):
    """The roaming exit: its capability is probed again, conntrack entries of
    the old outer address end the connection they carried (T sees another
    source), new flows leave through the new uplink, and the role stays."""
    routed = cell["routedAt"]
    samples = cell.get("exitCapability") or []

    def status_of(value):
        capability = (value or {}).get("server") or {}
        status = capability.get("status")
        return status.get("status") if isinstance(status, dict) else status

    timeline = [{"atS": round(sample["at"] - routed, 3), "status": status_of(sample["value"]),
                 "installed": ((sample["value"] or {}).get("server") or {}).get("installedFamilies")}
                for sample in samples]
    probing = next((item["atS"] for item in timeline if item["status"] == "probing" and item["atS"] >= -0.5), None)
    ready = next((item["atS"] for item in timeline
                  if probing is not None and item["status"] == "ready" and item["atS"] > probing), None)
    final = timeline[-1] if timeline else {}
    t_client = parse_client(report / "b" / f"traffic-{phase}-t-{name}" / "client.log")
    late_echo = [at for at, *_ in t_client["techo"] if at >= routed + 3]
    ended = [event for event in t_client["terror"] + t_client["tclosed"] if event[0] >= routed - 0.5]
    udp_after = [at for at, _ in t_client["uecho"] if at >= routed]
    probe = cell.get("exitProbe") or {}
    peers = []
    path = report / "t" / "target" / "tcp-peers.log"
    if path.exists():
        peers = [line.split() for line in path.read_text().splitlines() if len(line.split()) == 2]
    expected = manifest["addresses"]["wan"]["router-a2" if name == "soft" else "router-a1"]
    # The new TCP flow's source as T saw it: the peer T accepted during the probe.
    window = (probe.get("startedAtNs", 0), probe.get("endedAtNs", 0))
    probed = [peer for at, peer in peers if window[0] <= int(at) <= window[1]]
    last_peer = probed[-1] if probed else None
    stopped = [event for event in events if str(event.get("message", "")).startswith(EXIT_FAILURES)]
    measured["exit"] = {"capability": timeline[:32], "probingS": probing, "readyS": ready,
                        "existingTcpEnded": [(round(at - routed, 3), *rest) for at, *rest in ended][:4],
                        "existingTcpEchoesAfter3s": len(late_echo),
                        "existingUdpFirstReplyS": round(min(udp_after) - routed, 3) if udp_after else None,
                        "newFlows": probe, "tLastTcpPeer": last_peer, "expectedSource": expected,
                        "roleEvents": stopped[:8]}
    return {
        "exitProbing": probing is not None,
        "exitReadyAgain": ready is not None and ready <= EXIT_READY_LIMIT
        and "ipv4" in [str(family).lower() for family in (final.get("installed") or [])],
        "existingExitTcpEnds": bool(ended) or not late_echo,
        "newExitFlows": probe.get("tcp") is True and probe.get("udp") is True,
        "newSourceAtT": bool(last_peer) and last_peer.rsplit(":", 1)[0] == expected,
        "exitRoleKept": not stopped,
    }


def relay_checks(report, cell, measured, phase):
    """The relay cell's own evidence: the old direct flow waits exactly for the
    re-traversal, new flows in that gap go through R, a flow started on R stays
    there, and a flow started after the re-traversal does not cross R."""
    routed = cell["routedAt"]
    recovered = cell.get("recoveredAt")
    left = cell.get("directLeftAt")
    flows = parse_client(report / "a" / f"traffic-{phase}-flows" / "client.log")
    samples = []
    for line in (report / "r" / "r-rx.log").read_text().splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[1].isdigit():
            samples.append((int(fields[0]) / 1e9, int(fields[1])))
    reacquired = measured["reacquiredS"]
    in_gap = {identifier: at for identifier, at in flows["fsend"].items()
              if left is not None and recovered is not None and left + 0.25 <= at <= recovered - 0.25}
    answered_in_gap = {identifier for identifier, at in in_gap.items()
                       if flows["fecho"].get(identifier) is not None and flows["fecho"][identifier] < recovered
                       and flows["fecho"][identifier] - at <= 1.0}
    relay_stream = parse_client(report / "a" / f"traffic-relay-{cell['cell']}" / "client.log")
    after_stream = parse_client(report / "a" / f"traffic-after-{cell['cell']}" / "client.log")
    relay_started, after_started = cell.get("relayStreamAt"), cell.get("afterStreamAt")
    after_stopped = cell.get("afterStreamStoppedAt")
    measured["relay"] = {
        "directLeftS": round(left - routed, 3) if left else None,
        "recoveredS": round(recovered - routed, 3) if recovered else None,
        "newFlowsInGap": len(in_gap), "newFlowsAnsweredThroughR": len(answered_in_gap),
        "relayStreamLongestGapS": round(longest_gap(relay_started + 1, cell["relayStreamStoppedAt"],
                                                    [at for at, _ in relay_stream["uecho"]]), 3)
        if relay_started else None,
        "rRateBeforeRelayStream": rate(samples, routed - 3, relay_started) if relay_started else None,
        "rRateRelayStreamAfterRecovery": rate(samples, recovered + 1, after_started) if after_started else None,
        "rRateWithAfterStream": rate(samples, after_started + 0.5, after_stopped) if after_started else None,
        "afterStreamAnswered": sum(1 for _ in after_stream["uecho"]),
    }
    relay = measured["relay"]
    stays = relay["rRateRelayStreamAfterRecovery"]
    added = (relay["rRateWithAfterStream"] - stays) if stays is not None and relay["rRateWithAfterStream"] is not None else None
    relay["rRateAddedByAfterStream"] = round(added, 1) if added is not None else None
    return {
        "gapIsReacquisition": reacquired is not None and measured["udpLongestGapS"] <= reacquired + 1.0,
        # Every per-datagram flow of the gap answered through R within 1 s;
        # the relay stream (also started in the gap) shows that R carried it.
        "newFlowsThroughR": answered_in_gap == set(in_gap) and relay["rRateBeforeRelayStream"] is not None,
        "relayFlowStaysOnR": stays is not None and stays >= 150 and relay["relayStreamLongestGapS"] is not None
        and relay["relayStreamLongestGapS"] <= 1.0,
        "afterFlowDirect": added is not None and added < 50 and relay["afterStreamAnswered"] > 100,
    }


def flap_checks(cell, measured, events, b_events, manifest):
    """Every path change is one complete reaction (changes within the 250 ms
    coalescing count once); candidate reports stay bounded; B's channel ends
    at A's final source and no circuit ends."""
    classified = [event for event in events if event.get("message") == "Underlay network change classified"
                  and event.get("class") in ("path", "Path")]
    reactions = [event for event in events if event.get("message") == "Connectivity roam"
                 and event.get("cause") == "path_changed"]
    reports = [event for event in events if event.get("message") == "State candidate publication acknowledged"]
    final = manifest["addresses"]["wan"][cell["finalRouter"]]
    endpoint = measured["bEndpointOfA"]["after"] or ""
    moves = [event for event in b_events if event.get("cause") == "source_moved"]
    bound = len(classified) * REPORTS_PER_PATH_CHANGE + 1
    measured["flap"] = {"steps": len(cell["steps"]), "pathChanges": len(classified), "reactions": len(reactions),
                        "candidateReports": len(reports), "candidateReportBound": bound,
                        "bSourceMoves": len(moves), "bFinalEndpoint": endpoint, "expectedSource": final}
    changes = (1, 1) if measured.get("cell") == "burst" else (1, len(cell["steps"]))
    return {
        "oneReactionPerPathChange": len(classified) == len(reactions) and changes[0] <= len(classified) <= changes[1],
        "candidateReportsBounded": len(reports) <= bound,
        "noCircuitRetired": measured["circuitsRetired"] == 0,
        "followsFinalSource": endpoint.startswith(final + ":"),
    }
