"""Roaming rows: A changes uplinks under live Mesh traffic. The daemons own
every reaction; this supervisor only changes A's underlay and measures.

Topology (every network an owned internal bridge):

    A ─ uplink1 ─ router-a1 (SNAT) ┐
    A ─ uplink2 ─ router-a2 (SNAT) ├─ wan ─ controller, B (direct, flap), C, R (relay)
    A ─ lan ─ C                    │
    B ─ bnet ─ router-b (SNAT) ────┘        (nat, relay)

Every router maps its actor port-preservingly and admits only replies of
flows the actor started (port-restricted filtering).

- `direct`: two phases (A dials B; B dials A at router-a1's WAN address,
  which forwards UDP 35000 to A), each with two cells: soft (default route to
  router-a2, then uplink1's address deleted) and hard (uplink2 disconnected,
  uplink1 connected again and routed).
- `nat`: A and B both behind NAT, the resident A–B channel a coordinated
  traversal; the same two cells. Each new uplink is a new mapping B's NAT
  never saw, so only a new traversal joins them again.
- `relay`: `nat` plus a public R both A and B keep channels to; during the
  re-traversal new flows go through R, flows already there stay there.
- `flap`: A dials public B; A switches uplinks twice within 2 s (soft,
  default route only), three times within microseconds (burst: one path
  change after coalescing) and five times (hard, uplink1's link down and up).
"""
import ipaddress
import json
import shutil
import subprocess
import time
from datetime import datetime

from node_lab import ROOT, run, sha256
from validation_support import bind_access_options, export_evidence
from verification_sources import fingerprint

NETWORKS = ("wan", "uplink1", "uplink2", "lan")
SCENARIOS = {
    "direct": {"phases": ("dialer", "listener"), "cells": ("soft", "hard"), "nodes": ("a", "b", "c")},
    "nat": {"phases": ("traversal",), "cells": ("soft", "hard"), "nodes": ("a", "b", "c")},
    "relay": {"phases": ("traversal",), "cells": ("soft", "hard"), "nodes": ("a", "b", "c", "r")},
    "flap": {"phases": ("dialer",), "cells": ("soft", "burst", "hard"), "nodes": ("a", "b", "c")},
}
# D14: with the peer directly reachable, the first inner reply within 3 s of
# the new uplink's route, on the same circuit; behind two NATs (a new
# coordinated traversal), within 5 s.
FIRST_REPLY_LIMIT = {"direct": 3.0, "flap": 3.0, "nat": 5.0, "relay": 5.0}
# Traffic observed after the route before a cell is judged.
SETTLE = {"direct": 12.0, "flap": 12.0, "nat": 25.0, "relay": 25.0}
# The flap cells' switches, FLAP_SPACING apart (at most 2 s in all); the
# burst cell's three switches are one shell command, well inside the 250 ms
# coalescing, and count as one path change.
FLAPS = {"soft": 2, "burst": 3, "hard": 5}
FLAP_SPACING = 0.4
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
        if message in EVENT_MESSAGES or fields.get("operation") in ("roam", "probe_recovery"):
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


def execute(report, artifacts, runtime_os, traversal_os, platform, short, load, scenario="direct"):
    plan = SCENARIOS[scenario]
    nodes_of_row = plan["nodes"]
    nat = scenario in ("nat", "relay")
    routers = ("router-a1", "router-a2", *(("router-b",) if nat else ()))
    roles = ("controller", *routers, *nodes_of_row)
    networks_of_row = NETWORKS + (("bnet",) if nat else ())
    identity = report.name
    label_key = "io.meshway.runtime-fixture"
    label = label_key + "=" + identity
    image = runtime_os["imageId"]
    router_image = traversal_os["imageId"]
    containers, networks, addresses = {}, {}, {}
    manifest = {**artifacts, "run": identity, "passed": False, "status": "preparing",
                "scope": "diagnostic-short" if short else "acceptance", "traversal": "roam", "scenario": scenario,
                "dnsBackend": "off", "runtimeOsAssets": runtime_os, "traversalOsAssets": traversal_os,
                "loadBeforeRuntime": load, "resources": {"containers": containers, "networks": networks},
                "cleanup": {}, "processes": {}, "cells": {}, "firstReplyLimitS": FIRST_REPLY_LIMIT[scenario]}
    metadata = report / "report.json"
    started = time.monotonic()
    failure = None

    def save():
        write(metadata, manifest)

    def owned(kind, identifier):
        command = ["docker", "inspect", identifier] if kind == "container" else ["docker", "network", "inspect", identifier]
        data = json.loads(run(*command, capture=True))[0]
        labels = data["Config"]["Labels"] if kind == "container" else data["Labels"]
        if labels.get(label_key) != identity:
            raise RuntimeError("roam fixture ownership changed")
        return data

    def command(role, *arguments, required=True, timeout=20):
        owned("container", containers[role])
        result = subprocess.run(["docker", "exec", containers[role], *arguments],
                                text=True, capture_output=True, timeout=timeout, close_fds=True)
        record = {"command": list(arguments), "exitCode": result.returncode,
                  "stdout": result.stdout, "stderr": result.stderr}
        if required and result.returncode:
            raise RuntimeError(f"roam fixture command failed on {role}: {' '.join(arguments)}: {result.stderr.strip()}")
        return record

    def detached(role, *arguments):
        owned("container", containers[role])
        subprocess.run(["docker", "exec", "-d", containers[role], *arguments], check=True)

    def clock(role="a"):
        return int(command(role, "date", "+%s%N")["stdout"].strip()) / 1e9

    def read_json(role, path):
        record = command(role, "cat", path, required=False)
        if record["exitCode"]:
            return None
        try:
            return json.loads(record["stdout"])
        except ValueError:
            return None

    def wait_for(predicate, description, timeout=90, interval=.2):
        deadline = time.monotonic() + timeout
        while True:
            value = predicate()
            if value:
                return value
            if time.monotonic() >= deadline:
                raise RuntimeError("roam stage timed out: " + description)
            for role in roles:
                if not owned("container", containers[role])["State"]["Running"]:
                    raise RuntimeError("roam actor stopped during " + description + ": " + role)
            time.sleep(interval)

    def signal(name):
        (report / "signals" / name).write_text("true\n")

    def status(role):
        return read_json(role, "/evidence/status.json") or {}

    def peer(role, node):
        return next((value for value in status(role).get("peers", []) if value.get("nodeId") == node), None)

    def state(role):
        return read_json("controller", f"/evidence/state-{role}.json")

    def report_of(view, node):
        entry = ((view or {}).get("nodes") or {}).get(node) or {}
        return ((entry.get("report") or {}).get("report")) or {}

    def snapshot(nodes):
        """Circuit and channel identities from the real Server State and A/B status."""
        views = {role: state(role) for role in nodes_of_row}
        result = {}
        for role in nodes_of_row:
            body = report_of(views[role], nodes[role])
            result[role] = {"paths": sorted(body.get("paths", {})),
                            "pathNodes": {key: value.get("nodes") for key, value in (body.get("paths") or {}).items()},
                            "links": {peer_id: sorted(link["linkId"] for link in links)
                                      for peer_id, links in (body.get("links") or {}).items()},
                            "sockets": {key: [endpoint.get("address") for endpoint in socket.get("endpoints") or []]
                                        for key, socket in (body.get("sockets") or {}).items()}}
        result["aStatus"] = {node: (peer("a", nodes[node]) or {}).get("endpoint") for node in ("b", "c")}
        result["bStatus"] = (peer("b", nodes["a"]) or {}).get("endpoint")
        return result

    def a_interface(address):
        links = json.loads(command("a", "ip", "-j", "address")["stdout"])
        names = [link["ifname"] for link in links for item in link["addr_info"] if item.get("local") == address]
        return names[0] if names else None

    def prefix(name):
        return ipaddress.ip_network(owned("network", networks[name])["IPAM"]["Config"][0]["Subnet"]).prefixlen

    def forget_holes(router, peer_address):
        """B's NAT forgets the replies it would admit from an old uplink: the
        hole a later return to that uplink would otherwise still find open."""
        record = command(router, "conntrack", "-D", "-p", "udp", "--reply-src", addresses["bnet"]["b"],
                         "--orig-src", peer_address, required=False)
        record.update(command(router, "conntrack", "-D", "-p", "udp", "--orig-src", addresses["bnet"]["b"],
                              "--orig-dst", peer_address, required=False))
        return record

    def route_a(router):
        return command("a", "/bin/sh", "-c", "ip route replace default via " + addresses[
            "uplink1" if router == "router-a1" else "uplink2"][router] + " && date +%s%N")

    def relay_window(cell, nodes, mesh, routed_at):
        """While A has no direct channel to B, start a UDP flow (through R);
        once A is direct again, a second one (direct). Both run alongside the
        old direct flows and the per-datagram new flows."""
        out = {"cell": cell}

        def towards_b():
            return (peer("a", nodes["b"]) or {}).get("state")
        deadline = time.monotonic() + 20
        while towards_b() == "direct" and time.monotonic() < deadline:
            time.sleep(.1)
        if towards_b() == "direct":
            out["directLeftAt"] = None
            time.sleep(SETTLE["relay"])
            return out
        out["directLeftAt"] = clock()
        relay_stop = f"/signals/roam-relay-stop-{cell}"
        detached("a", "python3", "/roam-fixture/traffic.py", "stream", mesh["b"], f"/evidence/traffic-relay-{cell}", relay_stop)
        out["relayStreamAt"] = clock()
        deadline = time.monotonic() + 40
        while towards_b() != "direct" and time.monotonic() < deadline:
            time.sleep(.1)
        out["recoveredAt"] = clock() if towards_b() == "direct" else None
        if out["recoveredAt"] is not None:
            time.sleep(2)
            after_stop = f"/signals/roam-after-stop-{cell}"
            detached("a", "python3", "/roam-fixture/traffic.py", "stream", mesh["b"], f"/evidence/traffic-after-{cell}",
                     after_stop)
            out["afterStreamAt"] = clock()
            time.sleep(4)
            signal(after_stop.rsplit("/", 1)[1])
            out["afterStreamStoppedAt"] = clock()
            time.sleep(2)
        signal(relay_stop.rsplit("/", 1)[1])
        out["relayStreamStoppedAt"] = clock()
        time.sleep(max(0.0, routed_at + SETTLE["relay"] - clock()))
        return out

    save()
    try:
        support = report / "roam-support"
        support.mkdir()
        for name in ("router.sh", "traffic.py"):
            shutil.copyfile(ROOT / "dev/roam-fixture" / name, support / name)
        for role in roles:
            (report / role).mkdir(exist_ok=True)
        preparation = ["docker", "run", "--rm", "--pull", "never", "--label", label,
                       "--network", "none", "--read-only", "--cap-drop", "ALL", *bind_access_options(),
                       "--security-opt", "no-new-privileges", "--entrypoint", "", "--tmpfs", "/state:mode=700",
                       "--mount", f"type=bind,source={report / 'bin'},target=/fixtures,readonly",
                       "--mount", f"type=bind,source={report / 'public'},target=/public",
                       "--mount", f"type=bind,source={report / 'controller'},target=/controller",
                       image, "/fixtures/meshway-runtime-fixture", "prepare"]
        run(*preparation, capture=True)
        for name in networks_of_row:
            identifier = run("docker", "network", "create", "--internal", "--label", label,
                             f"meshway-runtime-{identity.lower()}-roam-{name}", capture=True).strip()
            networks[name] = identifier
            save()
            inspection = owned("network", identifier)
            subnet = ipaddress.ip_network(inspection["IPAM"]["Config"][0]["Subnet"])
            if (not inspection["Internal"] or inspection["Driver"] != "bridge" or subnet.version != 4
                    or subnet.overlaps(ipaddress.ip_network("100.64.0.0/10"))):
                raise RuntimeError("roam network must be an internal IPv4 bridge outside the Mesh pool")
            base = subnet.network_address
            if name == "wan":
                on_wan = ("controller", "b", "c", "router-a1", "router-a2") if not nat else (
                    "controller", "c", "router-a1", "router-a2", "router-b", "r")
                addresses[name] = {role: str(base + 2 + index) for index, role in enumerate(on_wan)
                                   if role in roles}
            elif name == "lan":
                addresses[name] = {"a": str(base + 2), "c": str(base + 3)}
            elif name == "bnet":
                addresses[name] = {"router-b": str(base + 2), "b": str(base + 3)}
            else:
                router = "router-a1" if name == "uplink1" else "router-a2"
                addresses[name] = {router: str(base + 2), "a": str(base + 3)}
        manifest["addresses"] = addresses
        write(report / "public/roam-topology.json", {
            "scenario": scenario, "controller": addresses["wan"]["controller"],
            "b": addresses["bnet"]["b"] if nat else addresses["wan"]["b"],
            "cLan": addresses["lan"]["c"], "aPublished": None if nat else addresses["wan"]["router-a1"],
            "r": addresses["wan"]["r"] if "r" in roles else None})
        attachments = {"controller": ["wan"], "b": ["bnet"] if nat else ["wan"], "c": ["wan", "lan"], "r": ["wan"],
                       "router-a1": ["wan", "uplink1"], "router-a2": ["wan", "uplink2"], "router-b": ["wan", "bnet"],
                       "a": ["lan", "uplink1", "uplink2"]}
        for role in roles:
            first = attachments[role][0]
            router = role.startswith("router-")
            creation = ["docker", "create", "--pull", "never", "--name", f"meshway-runtime-{identity.lower()}-{role}",
                        "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                        "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "768m", "--pids-limit", "128",
                        "--tmpfs", "/state:rw,nosuid,nodev,mode=700", "--entrypoint", "",
                        "--network", networks[first], "--ip", addresses[first][role],
                        "--sysctl", "net.ipv4.ip_forward=" + ("1" if router else "0"),
                        "--sysctl", "net.ipv4.conf.all.rp_filter=0", "--sysctl", "net.ipv4.conf.default.rp_filter=0",
                        "--env", "TMPDIR=/state", "--env", "MESHWAY_ROAM_FIXTURE_OWNED=1", *bind_access_options()]
            if router:
                creation += ["--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW",
                             "--tmpfs", "/run:rw,nosuid,nodev,mode=700"]
            elif role != "controller":
                creation += ["--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW", "--device", "/dev/net/tun:/dev/net/tun",
                             "--sysctl", "net.ipv6.conf.all.disable_ipv6=0", "--sysctl", "net.ipv6.conf.default.disable_ipv6=0"]
            mounts = [("bin", "/fixtures", True), (role, "/evidence", False), ("signals", "/signals", False),
                      ("public", "/public", True), ("roam-support", "/roam-fixture", True)]
            if role == "controller":
                mounts.append(("controller", "/controller", True))
            for source, target, readonly in mounts:
                creation += ["--mount", f"type=bind,source={report / source},target={target}" + (",readonly" if readonly else "")]
            if router:
                inside = {"router-a1": "uplink1", "router-a2": "uplink2", "router-b": "bnet"}[role]
                actor = "b" if role == "router-b" else "a"
                mode = "forward" if role == "router-a1" and not nat else "nat"
                creation += [router_image, "/bin/sh", "/roam-fixture/router.sh", addresses[inside][role],
                             addresses["wan"][role], addresses[inside][actor], mode]
            else:
                # A long lease: circuit renewal at a 60 s lease end is not roaming.
                creation += [image, "/fixtures/meshway-runtime-fixture", "--log-format", "json",
                             "--transport", "native-udp", "--traversal", "roam", "--netmap-validity", "1800s",
                             "roam-actor", "--role", role]
            containers[role] = run(*creation, capture=True).strip()
            manifest["resources"].setdefault("commands", {})[role] = creation
            save()
            for name in attachments[role][1:]:
                run("docker", "network", "connect", "--ip", addresses[name][role], networks[name], containers[role], capture=True)
            run("docker", "start", containers[role], capture=True)
        deadline = time.monotonic() + 30
        for role in roles:
            ready = "/evidence/ready.json" if role.startswith("router-") else "/evidence/ready"
            while command(role, "test", "-f", ready, required=False)["exitCode"]:
                if time.monotonic() >= deadline or not owned("container", containers[role])["State"]["Running"]:
                    raise RuntimeError("roam actor readiness failed: " + role)
                time.sleep(.1)
        # A's only default route: uplink1's router; B's (behind NAT) its own.
        # Internal bridges add none.
        command("a", "ip", "route", "replace", "default", "via", addresses["uplink1"]["router-a1"])
        if nat:
            command("b", "ip", "route", "replace", "default", "via", addresses["bnet"]["router-b"])
        write(report / "a/routes-initial.json", json.loads(command("a", "ip", "-j", "route")["stdout"]))
        if "r" in roles:
            # R's received-packet counter: what crosses R, sampled 4/s.
            detached("r", "/bin/sh", "-c", "while test ! -f /signals/roam-stop; do printf '%s %s\\n' "
                     "\"$(date +%s%N)\" \"$(cat /sys/class/net/eth0/statistics/rx_packets)\" >> /evidence/r-rx.log; "
                     "sleep 0.25; done")
        manifest["status"] = "enrolling"
        save()
        signal("roam-start")
        for phase in plan["phases"]:
            configured = wait_for(lambda: read_json("controller", f"/signals/roam-configured-{phase}.json"),
                                  f"{phase} configuration", 180)
            nodes, mesh = configured["nodes"], configured["mesh"]
            manifest["nodes"], manifest["mesh"] = nodes, mesh
            manifest["status"] = phase
            save()
            stop = f"/signals/roam-traffic-stop-{phase}"

            def direct():
                return all((peer("a", nodes[node]) or {}).get("state") == "direct" for node in ("b", "c")) and (
                    "r" not in roles or (peer("a", nodes["r"]) or {}).get("state") == "direct")
            wait_for(direct, f"{phase} A–B and A–C channels", 120)
            for role in ("b", "c"):
                detached(role, "python3", "/roam-fixture/traffic.py", "serve", mesh[role], f"/evidence/traffic-{phase}", stop)
            time.sleep(1)
            for role in ("b", "c"):
                detached("a", "python3", "/roam-fixture/traffic.py", "client", mesh[role], f"/evidence/traffic-{phase}-{role}", stop)
            if scenario == "relay":
                detached("a", "python3", "/roam-fixture/traffic.py", "flows", mesh["b"], f"/evidence/traffic-{phase}-flows", stop)
            wait_for(lambda: read_json("a", f"/evidence/traffic-{phase}-b/started.json"), "A traffic", 20)

            def flowing():
                # Both peers answer before any perturbation: the cell measures
                # roaming, not the phase's content change.
                return all(command("a", "grep", "-q", "^uecho", f"/evidence/traffic-{phase}-{role}/client.log",
                                   required=False)["exitCode"] == 0 for role in ("b", "c"))
            wait_for(flowing, f"{phase} traffic answered by B and C", 60)
            time.sleep(5)
            if nat:
                # The direct circuit carries the flows: A–B is a traversal channel.
                wait_for(lambda: (peer("a", nodes["b"]) or {}).get("state") == "direct", "A–B direct before roaming", 30)
            for cell in plan["cells"]:
                name = f"{phase}/{cell}"
                record = {"before": snapshot(nodes)}
                if scenario == "flap":
                    # Both uplinks attached: soft switches only the default
                    # route; hard takes uplink1's link down and up.
                    record["perturbedAt"] = clock()
                    steps = []
                    uplink1 = a_interface(addresses["uplink1"]["a"])
                    if cell == "burst":
                        a1, a2 = addresses["uplink1"]["router-a1"], addresses["uplink2"]["router-a2"]
                        routed = command("a", "/bin/sh", "-c", f"ip route replace default via {a2} && ip route replace "
                                         f"default via {a1} && ip route replace default via {a2} && date +%s%N")
                        at = int(routed["stdout"].strip()) / 1e9
                        steps = [{"towards": towards, "at": at} for towards in ("router-a2", "router-a1", "router-a2")]
                    for step in range(FLAPS[cell] if cell != "burst" else 0):
                        towards_a2 = step % 2 == 0
                        if cell == "soft":
                            routed = route_a("router-a2" if towards_a2 else "router-a1")
                        else:
                            # The connected route returns with the link; the
                            # default route may need a few tries after it.
                            via = addresses["uplink2"]["router-a2"] if towards_a2 else addresses["uplink1"]["router-a1"]
                            routed = command("a", "/bin/sh", "-c", f"ip link set dev {uplink1} "
                                             f"{'down' if towards_a2 else 'up'} && ok= && for i in 1 2 3 4 5 6 7 8 9 10; "
                                             f"do if ip route replace default via {via}; then ok=1; break; fi; "
                                             "sleep 0.05; done && test -n \"$ok\" && date +%s%N")
                        steps.append({"towards": "router-a2" if towards_a2 else "router-a1",
                                      "at": int(routed["stdout"].strip()) / 1e9})
                        time.sleep(FLAP_SPACING)
                    record["steps"] = steps
                    record["routedAt"] = steps[-1]["at"]
                    record["finalRouter"] = steps[-1]["towards"]
                elif cell == "soft":
                    record["perturbedAt"] = clock()
                    record["routedAt"] = int(route_a("router-a2")["stdout"].strip()) / 1e9
                    interface = a_interface(addresses["uplink1"]["a"])
                    command("a", "ip", "address", "del", addresses["uplink1"]["a"] + "/" + str(prefix("uplink1")),
                            "dev", interface)
                    record["addressRemovedAt"] = clock()
                else:
                    record["perturbedAt"] = clock()
                    run("docker", "network", "disconnect", networks["uplink2"], containers["a"], capture=True)
                    record["disconnectedAt"] = clock()
                    run("docker", "network", "connect", "--ip", addresses["uplink1"]["a"], networks["uplink1"],
                        containers["a"], capture=True)
                    record["connectedAt"] = clock()
                    record["routedAt"] = int(route_a("router-a1")["stdout"].strip()) / 1e9
                record["routes"] = json.loads(command("a", "ip", "-j", "route")["stdout"])
                if scenario == "relay":
                    record.update(relay_window(cell, nodes, mesh, record["routedAt"]))
                else:
                    time.sleep(SETTLE[scenario])
                record["observedUntil"] = clock()
                record["after"] = snapshot(nodes)
                manifest["cells"][name] = record
                save()
                if cell == "soft" and scenario != "flap":
                    # The address-less uplink1 leaves; hard reconnects it.
                    run("docker", "network", "disconnect", networks["uplink1"], containers["a"], capture=True)
                    if nat:
                        record["forgotHoles"] = forget_holes("router-b", addresses["wan"]["router-a1"])
                        save()
                    time.sleep(3)
            if scenario != "flap":
                # Back to the starting attachment for the next phase.
                run("docker", "network", "connect", "--ip", addresses["uplink2"]["a"], networks["uplink2"],
                    containers["a"], capture=True)
            (report / "signals" / stop.rsplit("/", 1)[1]).write_text("true\n")
            time.sleep(2)
            signal(f"roam-phase-done-{phase}")
        signal("roam-complete")
        manifest["status"] = "finishing"
        save()
    except BaseException as error:
        failure = error
        manifest["failure"] = f"{type(error).__name__}: {error}"
    finally:
        errors = []
        for phase in plan["phases"]:
            signal(f"roam-traffic-stop-{phase}")
        signal("roam-down")
        signal("roam-stop")
        for role, identifier in containers.items():
            try:
                owned("container", identifier)
                run("docker", "wait", identifier, capture=True, timeout=60)
                container_state = owned("container", identifier)["State"]
                manifest["processes"][role] = {key: container_state[key] for key in ("ExitCode", "OOMKilled", "Running")}
                (report / (role + ".log")).write_text(run("docker", "logs", identifier, capture=True))
                if container_state["Running"] or container_state["OOMKilled"] or container_state["ExitCode"] != 0:
                    errors.append("actor did not finish successfully: " + role)
            except Exception as error:
                errors.append(str(error))
            try:
                owned("container", identifier)
                run("docker", "rm", "-f", identifier, capture=True)
                manifest["cleanup"][role] = True
            except Exception as error:
                errors.append(str(error))
        for name, identifier in reversed(list(networks.items())):
            try:
                owned("network", identifier)
                run("docker", "network", "rm", identifier, capture=True)
                manifest["cleanup"]["network-" + name] = True
            except Exception as error:
                errors.append(str(error))
        try:
            export_evidence(report, image, platform, "runtime", identity)
        except Exception as error:
            errors.append(str(error))
        if failure is None and not errors:
            try:
                manifest["results"] = analyse(report, manifest)
            except Exception as error:
                errors.append(f"analysis: {type(error).__name__}: {error}")
        if fingerprint(ROOT) != artifacts["sourceDigest"]:
            errors.append("source changed during roam fixture run")
        if errors:
            manifest["cleanupErrors"] = errors
            manifest["status"] = "failed"
        elif failure is None:
            gate = manifest["results"]["gate"]
            manifest["passed"] = gate["passed"]
            manifest["status"] = "passed" if gate["passed"] else "gateFailed"
        manifest["runtimeElapsedSeconds"] = time.monotonic() - started
        save()
        print(report, flush=True)
    if failure:
        raise failure
    if not manifest["passed"]:
        raise RuntimeError(manifest["status"] + ": inspect private roam report")


def analyse(report, manifest):
    scenario = manifest.get("scenario", "direct")
    limit = FIRST_REPLY_LIMIT[scenario]
    results = {"cells": {}}
    failures = []
    nodes = manifest["nodes"]
    for name, cell in manifest["cells"].items():
        phase = name.split("/")[0]
        client = parse_client(report / "a" / f"traffic-{phase}-b" / "client.log")
        received = parse_received(report / "b" / f"traffic-{phase}" / "udp-received.log")
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
                                           "no circuit retired; B follows A's final source"}[scenario]}
    write(report / "roam-results.json", results)
    return results


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
