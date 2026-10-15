"""roam/direct: A changes uplinks under live Mesh traffic. The daemons own every
reaction; this supervisor only changes A's underlay and measures.

Topology (every network an owned internal bridge):

    A ─ uplink1 ─ router-a1 (SNAT, UDP 35000 → A) ┐
    A ─ uplink2 ─ router-a2 (SNAT)                ├─ wan ─ controller, B, C
    A ─ lan ─ C                                    ┘

Two phases (A dials B; B dials A at router-a1's WAN address), each with two
cells: soft (default route to router-a2, then uplink1's address deleted) and
hard (uplink2 disconnected, uplink1 connected again and routed).
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

NODES = ("a", "b", "c")
ROLES = ("controller", "router-a1", "router-a2", *NODES)
NETWORKS = ("wan", "uplink1", "uplink2", "lan")
PHASES = ("dialer", "listener")
# D14: with the peer directly reachable, the first inner reply within 3 s of
# the new uplink's route, on the same circuit.
FIRST_REPLY_LIMIT = 3.0
# Traffic observed after the route before a cell is judged, and the quiet
# time before the next perturbation.
SETTLE = 12.0


def write(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def parse_client(path):
    events = {"usend": {}, "uecho": [], "techo": [], "pecho": [], "terror": [], "tclosed": [], "tconnect": []}
    for line in path.read_text().splitlines():
        fields = line.split()
        if len(fields) < 2:
            continue
        kind, at = fields[0], int(fields[1]) / 1e9
        if kind == "usend":
            events["usend"][int(fields[2])] = at
        elif kind == "uecho":
            events["uecho"].append((at, int(fields[2])))
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
        if (message in ("Dataplane circuit retired", "Connectivity channel transition", "Connectivity roam",
                        "Underlay network change classified", "Known peer facts updated")
                or fields.get("operation") in ("roam", "probe_recovery")):
            records.append({"at": at, **fields})
    return records


def measure(cell, client, received, t_route, end):
    """First replies after the new route, loss and the TCP stall of one cell."""
    def first(arrivals):
        later = [at for at, *_ in arrivals if t_route <= at <= end]
        return round(min(later) - t_route, 3) if later else None

    sent = {seq: at for seq, at in client["usend"].items() if cell["perturbedAt"] <= at <= end - 2}
    echoed = {seq for at, seq in client["uecho"] if seq in sent}
    at_b = {seq for seq in sent if seq in received}
    echo_after = [(at, seq) for at, seq in client["uecho"] if at >= t_route and client["usend"].get(seq, 0) >= t_route]
    def longest(arrivals):
        # The window's edges count: no arrival at all is one stall to its end.
        times = [cell["perturbedAt"], *sorted(at for at in arrivals if cell["perturbedAt"] <= at <= end), end]
        return max(b - a for a, b in zip(times, times[1:]))

    gaps = [longest([at for at, *_ in client["techo"]])]
    udp_gaps = [longest([at for at, _ in client["uecho"]])]
    return {
        "firstUdpReplyS": round(min(at for at, _ in echo_after) - t_route, 3) if echo_after else None,
        "firstPingReplyS": first(client["pecho"]),
        "firstTcpEchoS": first(client["techo"]),
        "udpSent": len(sent),
        "udpLostToB": len(sent) - len(at_b),
        "udpLostFromB": len(at_b) - len(echoed),
        "udpLongestGapS": round(max(udp_gaps), 3) if udp_gaps else None,
        "tcpLongestGapS": round(max(gaps), 3) if gaps else None,
        "tcpErrors": [event for event in client["terror"] + client["tclosed"] if event[0] <= end],
    }


def execute(report, artifacts, runtime_os, traversal_os, platform, short, load):
    identity = report.name
    label_key = "io.meshway.runtime-fixture"
    label = label_key + "=" + identity
    image = runtime_os["imageId"]
    router_image = traversal_os["imageId"]
    containers, networks, addresses = {}, {}, {}
    manifest = {**artifacts, "run": identity, "passed": False, "status": "preparing",
                "scope": "diagnostic-short" if short else "acceptance", "traversal": "roam",
                "dnsBackend": "off", "runtimeOsAssets": runtime_os, "traversalOsAssets": traversal_os,
                "loadBeforeRuntime": load, "resources": {"containers": containers, "networks": networks},
                "cleanup": {}, "processes": {}, "cells": {}, "firstReplyLimitS": FIRST_REPLY_LIMIT}
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

    def wait_for(predicate, description, timeout=90):
        deadline = time.monotonic() + timeout
        while True:
            value = predicate()
            if value:
                return value
            if time.monotonic() >= deadline:
                raise RuntimeError("roam stage timed out: " + description)
            for role in ROLES:
                if not owned("container", containers[role])["State"]["Running"]:
                    raise RuntimeError("roam actor stopped during " + description + ": " + role)
            time.sleep(.2)

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
        views = {role: state(role) for role in ("a", "b", "c")}
        result = {}
        for role in ("a", "b", "c"):
            body = report_of(views[role], nodes[role])
            result[role] = {"paths": sorted(body.get("paths", {})),
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

    save()
    try:
        support = report / "roam-support"
        support.mkdir()
        for name in ("router.sh", "traffic.py"):
            shutil.copyfile(ROOT / "dev/roam-fixture" / name, support / name)
        for role in ROLES:
            (report / role).mkdir(exist_ok=True)
        preparation = ["docker", "run", "--rm", "--pull", "never", "--label", label,
                       "--network", "none", "--read-only", "--cap-drop", "ALL", *bind_access_options(),
                       "--security-opt", "no-new-privileges", "--entrypoint", "", "--tmpfs", "/state:mode=700",
                       "--mount", f"type=bind,source={report / 'bin'},target=/fixtures,readonly",
                       "--mount", f"type=bind,source={report / 'public'},target=/public",
                       "--mount", f"type=bind,source={report / 'controller'},target=/controller",
                       image, "/fixtures/meshway-runtime-fixture", "prepare"]
        run(*preparation, capture=True)
        for name in NETWORKS:
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
                addresses[name] = {role: str(base + 2 + index) for index, role in
                                   enumerate(("controller", "b", "c", "router-a1", "router-a2"))}
            elif name == "lan":
                addresses[name] = {"a": str(base + 2), "c": str(base + 3)}
            else:
                router = "router-a1" if name == "uplink1" else "router-a2"
                addresses[name] = {router: str(base + 2), "a": str(base + 3)}
        manifest["addresses"] = addresses
        write(report / "public/roam-topology.json", {
            "controller": addresses["wan"]["controller"], "b": addresses["wan"]["b"],
            "cLan": addresses["lan"]["c"], "aPublished": addresses["wan"]["router-a1"]})
        attachments = {"controller": ["wan"], "b": ["wan"], "c": ["wan", "lan"],
                       "router-a1": ["wan", "uplink1"], "router-a2": ["wan", "uplink2"],
                       "a": ["lan", "uplink1", "uplink2"]}
        for role in ROLES:
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
                upstream = "uplink1" if role == "router-a1" else "uplink2"
                creation += [router_image, "/bin/sh", "/roam-fixture/router.sh", addresses[upstream][role],
                             addresses["wan"][role], addresses[upstream]["a"]]
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
        for role in ROLES:
            ready = "/evidence/ready.json" if role.startswith("router-") else "/evidence/ready"
            while command(role, "test", "-f", ready, required=False)["exitCode"]:
                if time.monotonic() >= deadline or not owned("container", containers[role])["State"]["Running"]:
                    raise RuntimeError("roam actor readiness failed: " + role)
                time.sleep(.1)
        # A's only default route: uplink1's router. Internal bridges add none.
        command("a", "ip", "route", "replace", "default", "via", addresses["uplink1"]["router-a1"])
        write(report / "a/routes-initial.json", json.loads(command("a", "ip", "-j", "route")["stdout"]))
        manifest["status"] = "enrolling"
        save()
        signal("roam-start")
        uplink1 = {"interface": a_interface(addresses["uplink1"]["a"])}
        for phase in PHASES:
            configured = wait_for(lambda: read_json("controller", f"/signals/roam-configured-{phase}.json"),
                                  f"{phase} configuration", 180)
            nodes, mesh = configured["nodes"], configured["mesh"]
            manifest["nodes"], manifest["mesh"] = nodes, mesh
            manifest["status"] = phase
            save()
            stop = f"/signals/roam-traffic-stop-{phase}"

            def direct():
                return all((peer("a", nodes[node]) or {}).get("state") == "direct" for node in ("b", "c"))
            wait_for(direct, f"{phase} A–B and A–C channels", 120)
            for role in ("b", "c"):
                subprocess.run(["docker", "exec", "-d", containers[role], "python3", "/roam-fixture/traffic.py", "serve",
                                mesh[role], f"/evidence/traffic-{phase}", stop], check=True)
            time.sleep(1)
            for role in ("b", "c"):
                subprocess.run(["docker", "exec", "-d", containers["a"], "python3", "/roam-fixture/traffic.py", "client",
                                mesh[role], f"/evidence/traffic-{phase}-{role}", stop], check=True)
            wait_for(lambda: read_json("a", f"/evidence/traffic-{phase}-b/started.json"), "A traffic", 20)
            time.sleep(5)
            for cell in ("soft", "hard"):
                name = f"{phase}/{cell}"
                record = {"before": snapshot(nodes)}
                if cell == "soft":
                    record["perturbedAt"] = clock()
                    command("a", "ip", "route", "replace", "default", "via", addresses["uplink2"]["router-a2"])
                    record["routedAt"] = clock()
                    interface = a_interface(addresses["uplink1"]["a"])
                    command("a", "ip", "address", "del", addresses["uplink1"]["a"] + "/" +
                            str(ipaddress.ip_network(owned("network", networks["uplink1"])["IPAM"]["Config"][0]["Subnet"]).prefixlen),
                            "dev", interface)
                    record["addressRemovedAt"] = clock()
                else:
                    record["perturbedAt"] = clock()
                    run("docker", "network", "disconnect", networks["uplink2"], containers["a"], capture=True)
                    record["disconnectedAt"] = clock()
                    run("docker", "network", "connect", "--ip", addresses["uplink1"]["a"], networks["uplink1"],
                        containers["a"], capture=True)
                    record["connectedAt"] = clock()
                    route = command("a", "/bin/sh", "-c", "ip route replace default via " + addresses["uplink1"]["router-a1"]
                                    + " && date +%s%N")
                    record["routedAt"] = int(route["stdout"].strip()) / 1e9
                    uplink1["interface"] = a_interface(addresses["uplink1"]["a"])
                record["routes"] = json.loads(command("a", "ip", "-j", "route")["stdout"])
                time.sleep(SETTLE)
                record["observedUntil"] = clock()
                record["after"] = snapshot(nodes)
                manifest["cells"][name] = record
                save()
                if cell == "soft":
                    # The address-less uplink1 leaves; hard reconnects it.
                    run("docker", "network", "disconnect", networks["uplink1"], containers["a"], capture=True)
                    time.sleep(3)
            # Back to the starting attachment for the next phase.
            run("docker", "network", "connect", "--ip", addresses["uplink2"]["a"], networks["uplink2"], containers["a"], capture=True)
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
        signal("roam-traffic-stop-dialer")
        signal("roam-traffic-stop-listener")
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
        measured.update({
            "circuitsBefore": before["a"]["paths"], "circuitsAfter": after["a"]["paths"],
            "sameCircuit": bool(a_paths_kept) and before["a"]["paths"] == after["a"]["paths"],
            "circuitsRetired": len(retired),
            "aChannelToB": {"before": before["a"]["links"].get(nodes["b"]), "after": after["a"]["links"].get(nodes["b"])},
            "bChannelToA": {"before": before["b"]["links"].get(nodes["a"]), "after": after["b"]["links"].get(nodes["a"])},
            "aEndpointOfB": {"before": before["aStatus"]["b"], "after": after["aStatus"]["b"]},
            "bEndpointOfA": {"before": before["bStatus"], "after": after["bStatus"]},
            "aChannelToC": {"before": before["a"]["links"].get(nodes["c"]), "after": after["a"]["links"].get(nodes["c"])},
            "lanUdpLost": lan_measured["udpLostToB"] + lan_measured["udpLostFromB"],
            "lanUdpLongestGapS": lan_measured["udpLongestGapS"],
            "aSockets": {"before": before["a"]["sockets"], "after": after["a"]["sockets"]},
            "aRoamEvents": [event for event in events if event.get("message") != "Dataplane circuit retired"][:64],
            "bRoamEvents": b_events[:64],
        })
        measured["unrelatedChannelUnchanged"] = (measured["aChannelToC"]["before"] == measured["aChannelToC"]["after"]
                                                 and measured["aChannelToC"]["before"] is not None)
        first = measured["firstUdpReplyS"]
        passed = (first is not None and first <= FIRST_REPLY_LIMIT and measured["sameCircuit"]
                  and not measured["tcpErrors"] and measured["unrelatedChannelUnchanged"])
        measured["gatePassed"] = passed
        if not passed:
            failures.append(name)
        results["cells"][name] = measured
    results["gate"] = {"passed": not failures and len(results["cells"]) == 4, "failedCells": failures,
                       "rule": f"first inner UDP reply ≤ {FIRST_REPLY_LIMIT} s after the new route, same circuit, "
                               "TCP stream not reset, unrelated LAN channel unchanged"}
    write(report / "roam-results.json", results)
    return results
