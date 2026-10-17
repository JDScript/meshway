"""Roaming rows: A changes uplinks under live Mesh traffic. The daemons own
every reaction; this supervisor only changes A's underlay and measures.

Topology (every network an owned internal bridge):

    A ─ uplink1 ─ router-a1 (SNAT) ┐
    A ─ uplink2 ─ router-a2 (SNAT) ├─ wan ─ controller, B (direct, flap, exit), C, R (relay), T (exit)
    A ─ lan ─ C                    │
    B ─ bnet ─ router-b (SNAT) ────┘        (nat, relay)
    A ─ srv6 ─ controller                   (direct: A's Server path is IPv6)

Every router maps its actor port-preservingly and admits only replies of
flows the actor started (port-restricted filtering).

- `direct`: two phases (A dials B; B dials A at router-a1's WAN address,
  which forwards UDP 35000 to A), each with three cells: soft (default route
  to router-a2, then uplink1's address deleted), hard (uplink2 disconnected,
  uplink1 connected again and routed) and dual-stack (uplink2 attached again,
  then only the IPv4 default route to router-a2). A reaches the Server over
  IPv6 on its own network, so in dual-stack nothing of A's Server path, links
  or addresses changes: only the IPv4 route toward its peers (the OPS13 VM).
- `nat`: A and B both behind NAT, the resident A–B channel a coordinated
  traversal; the same two cells. Each new uplink is a new mapping B's NAT
  never saw, so only a new traversal joins them again.
- `relay`: `nat` plus a public R both A and B keep channels to; during the
  re-traversal new flows go through R, flows already there stay there.
- `flap`: A dials public B; A switches uplinks twice within 2 s (soft,
  default route only), three times within microseconds (burst: one path
  change after coalescing) and five times (hard, uplink1's link down and up).
- `exit`: A is B's exit toward T on the WAN (masquerade through the uplink A
  roams on); A dials public B; the soft and hard cells. B runs Mesh traffic to
  A and, per cell, a new exit flow set to T; after each cell one new TCP and
  UDP flow to T.
"""
import ipaddress
import importlib.util
import json
import shutil
import subprocess
import threading
import time
import uuid

from node_lab import ROOT, run
from validation_support import bind_access_options, export_evidence
from verification_sources import fingerprint

_spec = importlib.util.spec_from_file_location("meshway_roam_evidence", ROOT / "dev/roam-fixture/evidence.py")
evidence = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(evidence)
SCENARIOS, FIRST_REPLY_LIMIT, write, analyse = evidence.SCENARIOS, evidence.FIRST_REPLY_LIMIT, evidence.write, evidence.analyse

NETWORKS = ("wan", "uplink1", "uplink2", "lan")
# Traffic observed after the route before a cell is judged.
SETTLE = {"direct": 12.0, "flap": 12.0, "nat": 25.0, "relay": 25.0, "exit": 12.0}
# The flap cells' switches, FLAP_SPACING apart (at most 2 s in all); the
# burst cell's three switches are one shell command, well inside the 250 ms
# coalescing, and count as one path change.
FLAPS = {"soft": 2, "burst": 3, "hard": 5}
FLAP_SPACING = 0.4


class Sampler:
    """Distinct values of `read` (5/s, wall-clock stamped) until stopped."""

    def __init__(self, read):
        self.read, self.samples, self.done = read, [], threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.done.is_set():
            try:
                value = self.read()
            except Exception as error:
                value = {"error": str(error)}
            if not self.samples or self.samples[-1]["value"] != value:
                self.samples.append({"at": time.time(), "value": value})
            self.done.wait(0.2)

    def stop(self):
        self.done.set()
        self.thread.join(timeout=10)
        return self.samples


def execute(report, artifacts, runtime_os, traversal_os, platform, short, load, scenario="direct"):
    plan = SCENARIOS[scenario]
    nodes_of_row = plan["nodes"]
    nat = scenario in ("nat", "relay")
    dual = scenario == "direct"
    exit_row = scenario == "exit"
    routers = ("router-a1", "router-a2", *(("router-b",) if nat else ()))
    roles = ("controller", *routers, *nodes_of_row, *(("t",) if exit_row else ()))
    networks_of_row = NETWORKS + (("bnet",) if nat else ()) + (("srv6",) if dual else ())
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
            ipv6 = []
            if name == "srv6":
                salt = uuid.uuid4().hex
                ipv6 = ["--ipv6", "--subnet", f"2001:db8:{salt[:4]}:{salt[4:8]}::/64"]
            identifier = run("docker", "network", "create", "--internal", *ipv6, "--label", label,
                             f"meshway-runtime-{identity.lower()}-roam-{name}", capture=True).strip()
            networks[name] = identifier
            save()
            inspection = owned("network", identifier)
            if name == "srv6":
                # A's Server path only; Docker may add an IPv4 subnet besides.
                subnet6 = next((ipaddress.ip_network(config["Subnet"]) for config in inspection["IPAM"]["Config"]
                                if ipaddress.ip_network(config["Subnet"]).version == 6), None)
                if not inspection["Internal"] or inspection["Driver"] != "bridge" or str(subnet6) != ipv6[2]:
                    raise RuntimeError("srv6 must be the requested internal IPv6 bridge")
                addresses[name] = {"controller": str(subnet6.network_address + 2), "a": str(subnet6.network_address + 3)}
                continue
            subnet = ipaddress.ip_network(inspection["IPAM"]["Config"][0]["Subnet"])
            if (not inspection["Internal"] or inspection["Driver"] != "bridge" or subnet.version != 4
                    or subnet.overlaps(ipaddress.ip_network("100.64.0.0/10"))):
                raise RuntimeError("roam network must be an internal IPv4 bridge outside the Mesh pool")
            base = subnet.network_address
            if name == "wan":
                on_wan = ("controller", "b", "c", "router-a1", "router-a2", "t") if not nat else (
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
            "r": addresses["wan"]["r"] if "r" in roles else None,
            "controller6": addresses["srv6"]["controller"] if dual else None,
            "t": addresses["wan"]["t"] if exit_row else None})
        attachments = {"controller": ["wan", *(("srv6",) if dual else ())], "b": ["bnet"] if nat else ["wan"],
                       "c": ["wan", "lan"], "r": ["wan"], "t": ["wan"],
                       "router-a1": ["wan", "uplink1"], "router-a2": ["wan", "uplink2"], "router-b": ["wan", "bnet"],
                       "a": ["lan", "uplink1", "uplink2", *(("srv6",) if dual else ())]}
        for role in roles:
            first = attachments[role][0]
            router = role.startswith("router-")
            creation = ["docker", "create", "--pull", "never", "--name", f"meshway-runtime-{identity.lower()}-{role}",
                        "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                        "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "768m", "--pids-limit", "128",
                        "--tmpfs", "/state:rw,nosuid,nodev,mode=700", "--entrypoint", "",
                        "--network", networks[first], "--ip", addresses[first][role],
                        # A forwards as B's exit (`exit`); the role probes it.
                        "--sysctl", "net.ipv4.ip_forward=" + ("1" if router or (exit_row and role == "a") else "0"),
                        "--sysctl", "net.ipv4.conf.all.rp_filter=0", "--sysctl", "net.ipv4.conf.default.rp_filter=0",
                        "--env", "TMPDIR=/state", "--env", "MESHWAY_ROAM_FIXTURE_OWNED=1", *bind_access_options()]
            if router:
                creation += ["--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW",
                             "--tmpfs", "/run:rw,nosuid,nodev,mode=700"]
            elif role not in ("controller", "t"):
                creation += ["--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW", "--device", "/dev/net/tun:/dev/net/tun",
                             "--sysctl", "net.ipv6.conf.all.disable_ipv6=0", "--sysctl", "net.ipv6.conf.default.disable_ipv6=0"]
                if exit_row and role == "a":
                    creation += ["--sysctl", "net.netfilter.nf_conntrack_acct=0"]
            elif role == "controller" and dual:
                creation += ["--sysctl", "net.ipv6.conf.all.disable_ipv6=0", "--sysctl", "net.ipv6.conf.default.disable_ipv6=0"]
            mounts = [("bin", "/fixtures", True), (role, "/evidence", False), ("signals", "/signals", False),
                      ("public", "/public", True), ("roam-support", "/roam-fixture", True)]
            if role == "controller":
                mounts.append(("controller", "/controller", True))
            for source, target, readonly in mounts:
                creation += ["--mount", f"type=bind,source={report / source},target={target}" + (",readonly" if readonly else "")]
            if role == "t":
                # A plain WAN service: UDP/TCP echo, recording its peers.
                creation += [image, "/bin/sh", "-c", "touch /evidence/ready && exec python3 /roam-fixture/traffic.py "
                             f"serve {addresses['wan']['t']} /evidence/target /signals/roam-stop"]
            elif router:
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
                flag = "--ip6" if name == "srv6" else "--ip"
                run("docker", "network", "connect", flag, addresses[name][role], networks[name], containers[role], capture=True)
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
        topology_t = addresses["wan"].get("t")
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
            # Who serves and who sends: A sends to B and C; in `exit` B sends
            # to A (its exit) and A to C.
            pairs = (("b", "a"), ("a", "c")) if exit_row else (("a", "b"), ("a", "c"))
            for _, server in pairs:
                detached(server, "python3", "/roam-fixture/traffic.py", "serve", mesh[server], f"/evidence/traffic-{phase}", stop)
            time.sleep(1)
            for client, server in pairs:
                detached(client, "python3", "/roam-fixture/traffic.py", "client", mesh[server], f"/evidence/traffic-{phase}-{server}", stop)
            if scenario == "relay":
                detached("a", "python3", "/roam-fixture/traffic.py", "flows", mesh["b"], f"/evidence/traffic-{phase}-flows", stop)
            wait_for(lambda: read_json(pairs[0][0], f"/evidence/traffic-{phase}-{pairs[0][1]}/started.json"), "traffic", 20)

            def answered(client, path):
                return command(client, "grep", "-q", "^uecho", path, required=False)["exitCode"] == 0

            def flowing():
                # Both peers answer before any perturbation: the cell measures
                # roaming, not the phase's content change.
                return all(answered(client, f"/evidence/traffic-{phase}-{server}/client.log") for client, server in pairs)
            wait_for(flowing, f"{phase} traffic answered by B and C", 60)
            time.sleep(5)
            if nat:
                # The direct circuit carries the flows: A–B is a traversal channel.
                wait_for(lambda: (peer("a", nodes["b"]) or {}).get("state") == "direct", "A–B direct before roaming", 30)
            for cell in plan["cells"]:
                name = f"{phase}/{cell}"
                exit_stop = f"/signals/roam-exit-stop-{cell}"
                if exit_row:
                    # A new set of exit flows to T for this cell: an exit
                    # connection the cell's roam ends, if any, is this one.
                    t_dir = f"/evidence/traffic-{phase}-t-{cell}"
                    detached("b", "python3", "/roam-fixture/traffic.py", "client", topology_t, t_dir, exit_stop)
                    wait_for(lambda: answered("b", t_dir + "/client.log") and command(
                        "b", "grep", "-q", "^techo", t_dir + "/client.log", required=False)["exitCode"] == 0,
                        f"{cell} exit flows to T answered", 60)
                    time.sleep(3)
                if cell == "dual-stack":
                    # Back to both uplinks (hard left uplink1 only); that
                    # attachment settles before the cell's single change.
                    run("docker", "network", "connect", "--ip", addresses["uplink2"]["a"], networks["uplink2"],
                        containers["a"], capture=True)
                    time.sleep(6)
                record = {"before": snapshot(nodes)}
                if dual:
                    record["aServerRoute"] = command("a", "ip", "-j", "route", "get", addresses["srv6"]["controller"],
                                                     required=False)["stdout"]
                sampler = Sampler(lambda: {"server": report_of(state("a"), nodes["a"]).get("exitCapability"),
                                           "status": status("a").get("exit")}) if exit_row else None
                if cell == "dual-stack":
                    record["perturbedAt"] = clock()
                    record["routedAt"] = int(route_a("router-a2")["stdout"].strip()) / 1e9
                elif scenario == "flap":
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
                if sampler:
                    record["exitCapability"] = sampler.stop()
                    # New flows once the cell settled, then this cell's old ones end.
                    probe_dir = f"/evidence/probe-{phase}-{cell}"
                    command("b", "python3", "/roam-fixture/traffic.py", "probe", topology_t, probe_dir, exit_stop,
                            timeout=30)
                    record["exitProbe"] = read_json("b", probe_dir + "/result.json")
                    signal(exit_stop.rsplit("/", 1)[1])
                manifest["cells"][name] = record
                save()
                if cell == "soft" and scenario != "flap":
                    # The address-less uplink1 leaves; hard reconnects it.
                    run("docker", "network", "disconnect", networks["uplink1"], containers["a"], capture=True)
                    if nat:
                        record["forgotHoles"] = forget_holes("router-b", addresses["wan"]["router-a1"])
                        save()
                    time.sleep(3)
            if dual:
                # dual-stack left both uplinks attached: route as at the start.
                route_a("router-a1")
                time.sleep(3)
            elif scenario != "flap":
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
        for cell in plan["cells"]:
            if exit_row:
                signal(f"roam-exit-stop-{cell}")
        signal("roam-down")
        # Nodes go down while the Server still answers: a Control request in
        # flight when it vanishes waits for its connect timeout, which the
        # actor's bounded `down` would not outlast (bounded, best effort).
        settle = time.monotonic() + 40
        while time.monotonic() < settle and not all(
                (report / "signals" / f"roam-down-{role}").exists() for role in nodes_of_row):
            time.sleep(.2)
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
