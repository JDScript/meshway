"""exit6/off: the exit topology dual-stack, E carrying IPv6 through NAT66.

The same Server, daemons, fixture actors and evidence of exit/off; only the
stage sequence differs. Stage numbers are those of docs/v2/node/validation.md
(exit6/off). Stage 9 (an exit prefix change) belongs to I6-2 and does not run.
No NAT, permit, flow or authority implementation lives here.
"""
import ipaddress
import json
import time

FAR_MTU = 1280
# A–R link MTU of the MSS stage: A's channel to R then carries inner IPv6
# packets of NARROW_MTU - 28 - 86 = 1286 bytes (MSS 1226), above the IPv6
# minimum; NO_IPV6_MTU leaves an inner limit below 1280, which carries no IPv6.
NARROW_MTU = 1400
NARROW_INNER = NARROW_MTU - 28 - 86
NO_IPV6_MTU = 1380
EACCES = 13


def write(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def family(capability, name):
    return next((entry for entry in (capability or {}).get("families") or [] if entry.get("family") == name), None)


def peer_address(peer):
    return peer.rsplit(":", 1)[0].strip("[]")


def run(ctx):
    report, manifest, command, status, wait_for = ctx.report, ctx.manifest, ctx.command, ctx.status, ctx.wait_for
    addresses, addresses6 = ctx.addresses, ctx.addresses6
    ext6, ext4 = addresses6["external"]["external"], addresses["external"]["external"]
    e_ext6 = addresses6["external"]["e"]
    far6 = addresses6["far"]["external"]
    lan6 = addresses6["lan"]["lan"]
    router6 = addresses6["external"]["router"]
    results = {}

    def stage(number, evidence, **detail):
        manifest["stages"][str(number)] = {"status": "passed", "evidence": evidence, **detail}
        ctx.save()

    def progress(name):
        manifest["status"] = name
        ctx.save()

    def capability():
        return ((status("e").get("exit") or {}).get("capability")) or {}

    def families_are(expected):
        value = status("e")
        cap = (value.get("exit") or {}).get("capability") or {}
        actual = {entry["family"]: (entry["status"], entry["installed"]) for entry in cap.get("families") or []}
        matched = actual == expected and ctx.running(value)
        if matched:
            results.setdefault("capabilities", []).append(cap)
        return matched

    ready = {"ipv4": ({"status": "ready"}, True), "ipv6": ({"status": "ready"}, True)}

    def a_cli(*arguments, as_json=True, timeout=45):
        record = command("a", "/fixtures/meshway", "--socket", "/state/node/control.sock",
                         *(["--json"] if as_json else []), *arguments, required=False, timeout=timeout)
        return record

    def exit_cli(profile):
        record = a_cli("exit", profile)
        if record["exitCode"]:
            write(report / "a" / "exit6-exit-failure.json", record)
            raise RuntimeError("meshway exit failed: " + profile)
        value = json.loads(record["stdout"])
        if value.get("applied") is not True or (value.get("local") or {}).get("appliedProfile") != profile:
            raise RuntimeError("the profile switch did not apply: " + profile)
        return value

    def a_plan(name):
        return json.loads(ctx.snapshot(name)["a"]["egressJournal"]["stdout"])["plan"]

    def contains(ranges, address):
        address = ipaddress.ip_address(address)
        return any(ipaddress.ip_address(start) <= address <= ipaddress.ip_address(end) for start, end in ranges)

    def tool(role, *arguments, timeout=30):
        record = command(role, "timeout", str(timeout), "python3", "/dns-fixture/icmp.py", *arguments,
                         required=False, timeout=timeout + 5)
        try:
            record["value"] = json.loads(record["stdout"].strip().splitlines()[-1])
        except (ValueError, IndexError):
            record["value"] = None
        return record

    def visits(peer, protocol="tcp", service="external"):
        record = command(service, "cat", "/evidence/access.json", required=False)
        values = json.loads(record["stdout"]) if record["exitCode"] == 0 else []
        return [v for v in values if v["protocol"] == protocol and peer_address(v["peer"]) == peer]

    def tcp_echo(target, size=4096):
        return tool("a", "tcp", target, "8080", str(size), timeout=25)

    def conntrack6(destination):
        return command("e", "/usr/sbin/conntrack", "-L", "-f", "ipv6", "-d", destination, "-o", "extended",
                       required=False)

    # Stage 1: capability. E acquires the IPv6 executor; its role carries
    # both families, each ready and installed; A captures IPv6 exit ranges.
    progress("exit6-capability")
    # E's IPv4 forwarding is its host administrator's precondition (exit/off
    # stage 2 observes it off first); IPv6 forwarding is on from creation.
    results["ipv4Forwarding"] = command("e", "/bin/sh", "/dns-fixture/forwarding.sh", "1")
    wait_for(lambda: families_are(ready), "E's IPv4 and IPv6 families ready and installed", 120)
    e_status = status("e")
    write(report / "e" / "exit6-installed-status.json", e_status)
    acquired = json.loads(command("controller", "cat", "/evidence/acquire-" + e_status["instance"]["instanceId"]
                                  + ".json")["stdout"])
    write(report / "e" / "exit6-acquisition.json", acquired)
    for feature in ("feature:exit-ipv4", "feature:exit-ipv6"):
        if acquired["capabilities"].get(feature) != {"kind": "feature", "version": 1}:
            raise RuntimeError("E did not acquire " + feature + " v1")
    wait_for(lambda: all(ctx.running(status(role)) for role in ("a", "r")), "A/R running", 60)
    before = ctx.snapshot("exit6-installed")
    plans = {role: json.loads(before[role]["netmap"]["stdout"])["netmap"] for role in ("a", "r", "e")}
    write(report / "exit6-compiled-plans.json", plans)
    if (plans["e"]["exitRole"] or {}).get("families") != ["ipv4", "ipv6"]:
        raise RuntimeError("E's role does not carry IPv4 and IPv6")
    if (plans["r"]["exitRole"] or {}).get("families") != ["ipv4"]:
        raise RuntimeError("R (E2) is not an IPv4-only exit")
    eligible = {item["node"]: item["families"] for item in plans["a"]["projection"].get("eligibleExits", [])}
    if eligible.get(plans["e"]["nodeId"]) != ["ipv4", "ipv6"]:
        raise RuntimeError("A's projection does not carry E's families")
    plan = json.loads(before["a"]["egressJournal"]["stdout"])["plan"]
    if (not contains(plan.get("managedIpv6", []), ext6) or contains(plan["sinkIpv6"], ext6)
            or not contains(plan.get("managedIpv6", []), lan6)):
        raise RuntimeError("A does not capture IPv6 exit ranges as managed")
    route = command("a", "ip", "-6", "-j", "route", "get", ext6, required=False)
    tuns = json.loads(before["a"]["tunReceived"]["stdout"])
    if route["exitCode"] or not any(item.get("dev") in tuns for item in json.loads(route["stdout"])):
        raise RuntimeError("A's IPv6 exit destination is not routed to its TUN")
    write(report / "exit6-capture.json", {"plan": {key: plan.get(key) for key in ("managedIpv6", "sinkIpv6")},
                                          "route": route, "summary": (status("a").get("netmap") or {}).get("capture")})
    stage(1, ["e/exit6-installed-status.json", "e/exit6-acquisition.json", "exit6-compiled-plans.json",
              "exit6-capture.json"])

    # Stage 3: IPv6 TCP, UDP and ICMPv6 echo through A→R→E, masqueraded to
    # E's external IPv6 address; the exit LAN by its explicit ULA grant.
    progress("exit6-traffic")
    traffic = {"tcp": tcp_echo(ext6), "udp": tool("a", "echo", ext6, "8080"),
               "ping": command("a", "ping", "-6", "-n", "-c", "3", "-W", "2", ext6, required=False),
               "lan": tcp_echo(lan6)}
    traffic["tcpVisits"] = visits(e_ext6)
    traffic["udpVisits"] = visits(e_ext6, "udp")
    traffic["lanVisits"] = visits(addresses6["lan"]["e"], service="lan")
    traffic["conntrack"] = conntrack6(ext6)
    write(report / "exit6-traffic.json", traffic)
    if (traffic["tcp"]["exitCode"] or not (traffic["tcp"]["value"] or {}).get("echoed")
            or not (traffic["udp"]["value"] or {}).get("echoed") or traffic["ping"]["exitCode"]
            or traffic["lan"]["exitCode"]):
        raise RuntimeError("IPv6 traffic through the exit failed")
    if not traffic["tcpVisits"] or not traffic["udpVisits"] or not traffic["lanVisits"]:
        raise RuntimeError("the service did not see E's masqueraded IPv6 address")
    if "mark=" not in traffic["conntrack"]["stdout"]:
        raise RuntimeError("E holds no owned IPv6 conntrack entry")
    stage(3, ["exit6-traffic.json"])

    # Stage 4: PMTU. The far link is 1280: a TCP echo converges and an
    # oversized DF packet is answered packet too big naming 1280 by the
    # router; E's own narrower external interface answers the same from E;
    # every traceroute hop's time exceeded returns to A.
    progress("exit6-pmtu")
    pmtu = {}
    narrow = [link["ifname"] for link in json.loads(command("router", "ip", "-j", "address")["stdout"])
              if any(item.get("local") == addresses6["far"]["router"] for item in link["addr_info"])]
    if len(narrow) != 1:
        raise RuntimeError("router far interface not found")
    pmtu["narrow"] = command("router", "ip", "link", "set", "dev", narrow[0], "mtu", str(FAR_MTU))
    pmtu["route"] = command("e", "ip", "-6", "route", "add", ctx.subnets6["far"], "via", router6)
    try:
        detached_listener(ctx, "/evidence/icmp6-pmtu.json", 90)
        pmtu["tcp"] = tool("a", "tcp", far6, "8080", "65536")
        pmtu["pingTooLarge"] = command("a", "ping", "-6", "-n", "-M", "do", "-s", "1260", "-c", "2", "-W", "2", far6,
                                       required=False)
        pmtu["routeCache"] = command("a", "ip", "-6", "route", "get", far6)
        pmtu["pingFits"] = command("a", "ping", "-6", "-n", "-M", "do", "-s", str(FAR_MTU - 48), "-c", "2", "-W", "2",
                                   far6, required=False)
        trace = command("a", "traceroute", "-6", "-I", "-n", "-q", "1", "-w", "2", "-m", "5", far6, required=False)
        hops = {}
        for line in trace["stdout"].splitlines()[1:]:
            fields = line.split()
            if len(fields) >= 2 and fields[0].isdigit():
                hops[int(fields[0])] = fields[1]
        pmtu["traceroute"] = {"record": trace, "hops": hops}
        e6 = {item["local"] for link in json.loads(command("e", "ip", "-j", "address")["stdout"])
              for item in link["addr_info"] if item["family"] == "inet6"}
        errors = json.loads(command("a", "cat", "/evidence/icmp6-pmtu.json")["stdout"])["errors"]
        pmtu["packetTooBig"] = [entry for entry in errors if entry["type"] == 2 and entry["mtu"] == FAR_MTU]
        if (pmtu["tcp"]["exitCode"] or not (pmtu["tcp"]["value"] or {}).get("echoed")
                or pmtu["pingTooLarge"]["exitCode"] == 0 or pmtu["pingFits"]["exitCode"]
                or f"mtu {FAR_MTU}" not in pmtu["routeCache"]["stdout"] or not pmtu["packetTooBig"]):
            raise RuntimeError("IPv6 PMTU discovery through the exit did not converge")
        if hops.get(1) not in e6 or hops.get(3) != far6 or 2 not in hops:
            raise RuntimeError("traceroute -6 through the exit did not show E, the router and the target: "
                               + json.dumps(hops))
        # E's own packet too big: its external interface narrower than A's packet.
        e_if = [link["ifname"] for link in json.loads(command("e", "ip", "-j", "address")["stdout"])
                if any(item.get("local") == e_ext6 for item in link["addr_info"])][0]
        pmtu["eNarrow"] = command("e", "ip", "link", "set", "dev", e_if, "mtu", "1300")
        try:
            detached_listener(ctx, "/evidence/icmp6-host.json", 30)
            pmtu["hostTooBig"] = command("a", "ping", "-6", "-n", "-M", "do", "-s", "1270", "-c", "2", "-W", "2", ext6,
                                         required=False)
            host = json.loads(command("a", "cat", "/evidence/icmp6-host.json")["stdout"])["errors"]
            pmtu["hostErrors"] = [entry for entry in host if entry["type"] == 2 and entry["mtu"] == 1300]
            if pmtu["hostTooBig"]["exitCode"] == 0 or not pmtu["hostErrors"]:
                raise RuntimeError("E's own packet too big did not reach A")
        finally:
            pmtu["eRestored"] = command("e", "ip", "link", "set", "dev", e_if, "mtu", "1500", required=False)
    finally:
        pmtu["routeRemoved"] = command("e", "ip", "-6", "route", "del", ctx.subnets6["far"], "via", router6,
                                       required=False)
        pmtu["restored"] = command("router", "ip", "link", "set", "dev", narrow[0], "mtu", "1500", required=False)
        write(report / "exit6-pmtu.json", pmtu)
    stage(4, ["exit6-pmtu.json"])

    # Stage 5: MSS. With the A–R link at NARROW_MTU, A's IPv6 SYN through the
    # exit is clamped to the channel's inner limit - 60; below the IPv6
    # minimum the channel carries no IPv6 and A answers itself (no route).
    progress("exit6-mss")
    mss = {}
    r_node = plans["r"]["nodeId"]
    links = {role: interface_of(command, role, addresses["ar"][role]) for role in ("a", "r")}

    def channel(value):
        peer = next((p for p in value.get("peers", []) if p.get("nodeId") == r_node), None)
        return (peer or {}).get("mtu") or {}

    try:
        for role, name in links.items():
            mss[role + "Narrowed"] = command(role, "ip", "link", "set", "dev", name, "mtu", str(NARROW_MTU))
        wait_for(lambda: channel(status("a")).get("inner") == NARROW_INNER, "A's channel to R at the narrow MTU")

        def clamped():
            tables = [line for line in command("a", "nft", "list", "tables")["stdout"].splitlines()
                      if line.startswith("table inet meshway_mss_")]
            if len(tables) != 1:
                return False
            mss["table"] = command("a", "nft", "list", "table", *tables[0].split()[1:])
            return (f"maxseg size set {NARROW_INNER - 60}" in mss["table"]["stdout"]
                    and "meta nfproto ipv6" in mss["table"]["stdout"])
        wait_for(clamped, "A's IPv6 MSS clamp through the exit", 10)
        mss["tcp"] = tcp_echo(ext6, 65536)
        if mss["tcp"]["exitCode"] or (mss["tcp"]["value"] or {}).get("mss", 1 << 16) > NARROW_INNER - 60:
            raise RuntimeError("A's IPv6 TCP through the exit was not clamped")
        for role, name in links.items():
            mss[role + "NoIpv6"] = command(role, "ip", "link", "set", "dev", name, "mtu", str(NO_IPV6_MTU))
        wait_for(lambda: (channel(status("a")).get("inner") or 1 << 16) < 1280, "A's channel below the IPv6 minimum")
        detached_listener(ctx, "/evidence/icmp6-noroute.json", 30)
        mss["noRoute"] = tool("a", "connect", ext6, "8080")
        errors = json.loads(command("a", "cat", "/evidence/icmp6-noroute.json")["stdout"])["errors"]
        mss["noRouteErrors"] = [entry for entry in errors if (entry["type"], entry["code"]) == (1, 0)]
        if (mss["noRoute"]["value"] or {}).get("errno") in (None, 0) or not mss["noRouteErrors"]:
            raise RuntimeError("an IPv6 exit flow over a channel below 1280 was not answered no route")
    finally:
        for role, name in links.items():
            mss[role + "Restored"] = command(role, "ip", "link", "set", "dev", name, "mtu", "1500", required=False)
    write(report / "exit6-mss.json", mss)
    stage(5, ["exit6-mss.json"])

    # Stage 8: AAAA and the trusted association. A name rule sends the name
    # zone through E (fallback direct); E's resolver answers the service's
    # IPv6 address as is, the association puts it in A's managed IPv6 and A
    # connects by name through E.
    progress("exit6-dns")
    dns = {}

    def stage_config(signal, done):
        (report / "signals" / signal).touch()
        wait_for(lambda: (report / "signals" / done).exists(), signal)
        revision = json.loads(command("controller", "cat", "/signals/" + done)["stdout"])
        def applied():
            for role in ("a", "r", "e"):
                value = status(role)
                if not ctx.running(value) or value["applied"]["revision"] != revision:
                    return False
            return True
        wait_for(applied, signal + " applied")
        return revision

    dns["configured"] = stage_config("exit-name-configure", "exit-name-configured.json")
    plan = a_plan("exit6-name-configured")
    if contains(plan.get("managedIpv6", []), ext6):
        raise RuntimeError("a name rule must own no IPv6 address before an association")
    name = "v6-" + report.name.lower() + ".name.exit-dns.test"
    deadline = time.monotonic() + 45
    while True:
        dns["lookup"] = ctx.lookup(name, "any")
        if ext6 in (dns["lookup"]["value"].get("addresses") or []):
            break
        if time.monotonic() >= deadline:
            raise RuntimeError("AAAA of the name zone was not returned through E")
        time.sleep(.5)
    plan = a_plan("exit6-associated")
    if not contains(plan.get("managedIpv6", []), ext6):
        raise RuntimeError("the AAAA association did not capture the IPv6 address")
    seen = len(visits(e_ext6))
    # By name over IPv6 alone (getaddrinfo would rank A's IPv4 first: its
    # Mesh IPv6 source is a ULA).
    dns["byName"] = command("a", "python3", "-c",
                            "import socket,sys; a=socket.getaddrinfo(sys.argv[1],8080,socket.AF_INET6,socket.SOCK_STREAM)[0][4]; "
                            "s=socket.create_connection(a[:2],3); s.settimeout(3); "
                            "s.sendall(b'exit6-name'); assert s.recv(64)==b'exit6-name'; print(s.getpeername()[0])",
                            name, required=False)
    if dns["byName"]["exitCode"] or dns["byName"]["stdout"].strip() != ext6 or len(visits(e_ext6)) <= seen:
        raise RuntimeError("A did not connect by name over IPv6 through E")
    dns["revoked"] = stage_config("exit-name-revoke", "exit-name-revoked.json")
    dns["restored"] = stage_config("exit-name-restore", "exit-name-restored.json")
    write(report / "exit6-dns.json", dns)
    stage(8, ["exit6-dns.json"])

    # Stage 10: switching to `mesh` ends every IPv6 exit flow; A's IPv6 exit
    # ranges leave the managed boundary.
    progress("exit6-switch")
    switch = {}
    holder = hold_flow(ctx, ext6, "/evidence/exit6-switch-flow.json", 30)
    wait_for(lambda: sum(ok for _, ok in (holder() or {}).get("tcp", [])) >= 3, "IPv6 exit flow before the switch", 30)
    switch["requested"] = time.time()
    switch["mesh"] = exit_cli("mesh")
    switch["switched"] = time.time()
    plan = a_plan("exit6-mesh")
    if plan.get("managedIpv6"):
        raise RuntimeError("the mesh profile still captures IPv6 exit ranges")
    wait_for(lambda: (holder() or {}).get("done"), "flow across the switch", 60)
    switch["flow"] = holder()
    after = [ok for at, ok in switch["flow"]["tcp"] if at > switch["switched"] + 1]
    if not after or any(after):
        raise RuntimeError("the switch did not end the IPv6 exit flow")
    switch["back"] = exit_cli("exit")
    write(report / "exit6-switch.json", switch)
    stage(10, ["exit6-switch.json"])

    # Stage 7: an exit with only IPv4. E's IPv6 family unavailable at run
    # time: A's IPv6 connect fails at once (EACCES) and IPv4 works; an exit
    # whose `exitFamilies` is IPv4 (R) sinks A's IPv6 statically.
    progress("exit6-ipv4-only")
    only = {"forwarding": command("e", "/bin/sh", "/dns-fixture/forwarding.sh", "ipv6", "0")}
    try:
        wait_for(lambda: families_are({"ipv4": ({"status": "ready"}, True),
                                       "ipv6": ({"status": "unavailable", "reason": "ipv6ForwardDisabled"}, True)}),
                 "E's IPv6 family unavailable", 90)

        def refused():
            only["connect6"] = tool("a", "connect", ext6, "8080")
            value = only["connect6"]["value"] or {}
            return value.get("errno") == EACCES
        wait_for(refused, "A's IPv6 connect refused at once", 60)
        if only["connect6"]["value"]["seconds"] > 1:
            raise RuntimeError("A's IPv6 connect was not refused at once")
        only["connect4"] = tool("a", "connect", ext4, "8080")
        if (only["connect4"]["value"] or {}).get("errno") != 0:
            raise RuntimeError("IPv4 through the exit failed while its IPv6 family was unavailable")
        only["ping"] = a_cli("ping", ext6, "-c", "1")
        if "exitUnavailable" not in only["ping"]["stdout"] + only["ping"]["stderr"]:
            raise RuntimeError("A's decision for IPv6 did not name exitUnavailable")
    finally:
        only["restored"] = command("e", "/bin/sh", "/dns-fixture/forwarding.sh", "ipv6", "1")
    wait_for(lambda: families_are(ready), "E's IPv6 family ready again", 90)
    only["e2"] = exit_cli("exit-ipv4")
    plan = a_plan("exit6-ipv4-only-exit")
    if contains(plan.get("managedIpv6", []), ext6) or not contains(plan["sinkIpv6"], ext6):
        raise RuntimeError("an IPv4-only exit did not sink A's IPv6 statically")
    only["e2Ping"] = a_cli("ping", ext6, "-c", "1")
    if "unsupportedFamily" not in only["e2Ping"]["stdout"] + only["e2Ping"]["stderr"]:
        raise RuntimeError("A's decision through an IPv4-only exit was not unsupportedFamily")
    only["e2Connect"] = tool("a", "connect", ext6, "8080")
    if (only["e2Connect"]["value"] or {}).get("errno") != EACCES:
        raise RuntimeError("the sunk IPv6 destination was not refused at once")
    only["back"] = exit_cli("exit")
    write(report / "exit6-ipv4-only.json", only)
    stage(7, ["exit6-ipv4-only.json"])

    # Stage 2: prerequisites. E's IPv6 forwarding off: only the IPv6 family
    # is unavailable and an IPv4 exit flow continues; restored, it is ready
    # again with the same installed role. A route a router advertisement
    # installed while forwarding ignores advertisements makes it unavailable.
    progress("exit6-prerequisites")
    prerequisites = {}
    holder = hold_flow(ctx, ext4, "/evidence/exit6-ipv4-flow.json", 40)
    wait_for(lambda: sum(ok for _, ok in (holder() or {}).get("tcp", [])) >= 3, "IPv4 exit flow", 30)
    role = (status("e").get("exit") or {}).get("installedRole")
    prerequisites["off"] = command("e", "/bin/sh", "/dns-fixture/forwarding.sh", "ipv6", "0")
    wait_for(lambda: families_are({"ipv4": ({"status": "ready"}, True),
                                   "ipv6": ({"status": "unavailable", "reason": "ipv6ForwardDisabled"}, True)}),
             "ipv6ForwardDisabled", 90)
    prerequisites["on"] = command("e", "/bin/sh", "/dns-fixture/forwarding.sh", "ipv6", "1")
    wait_for(lambda: families_are(ready), "IPv6 ready again", 90)
    if (status("e").get("exit") or {}).get("installedRole") != role:
        raise RuntimeError("E's role was reinstalled by an IPv6 prerequisite change")
    e_if = [link["ifname"] for link in json.loads(command("e", "ip", "-j", "address")["stdout"])
            if any(item.get("local") == e_ext6 for item in link["addr_info"])][0]
    prerequisites["acceptRa"] = command("e", "/bin/sh", "/dns-fixture/forwarding.sh", "accept-ra", e_if, "1")
    prerequisites["raRoute"] = command("e", "ip", "-6", "route", "add", "default", "via", router6, "dev", e_if,
                                       "proto", "ra")
    try:
        wait_for(lambda: families_are({"ipv4": ({"status": "ready"}, True),
                                       "ipv6": ({"status": "unavailable",
                                                 "reason": "ipv6RouterAdvertisementsIgnored"}, True)}),
                 "ipv6RouterAdvertisementsIgnored", 90)
    finally:
        prerequisites["raRemoved"] = command("e", "ip", "-6", "route", "del", "default", "via", router6, "dev", e_if,
                                             "proto", "ra", required=False)
        prerequisites["acceptRaRestored"] = command("e", "/bin/sh", "/dns-fixture/forwarding.sh", "accept-ra", e_if,
                                                    "0", required=False)
    wait_for(lambda: families_are(ready), "IPv6 ready after the advertised route", 90)
    wait_for(lambda: (holder() or {}).get("done"), "IPv4 flow across the IPv6 changes", 60)
    prerequisites["ipv4Flow"] = holder()
    if not all(ok for _, ok in prerequisites["ipv4Flow"]["tcp"]):
        raise RuntimeError("an IPv4 exit flow broke during IPv6 prerequisite changes")
    write(report / "exit6-prerequisites.json", prerequisites)
    stage(2, ["exit6-prerequisites.json"])

    # Stage 6: withdrawal and quarantine. Disabling E ends its IPv6 flows at
    # once and leaves no owned IPv6 conntrack; the retired reply tuple stays
    # quarantined and the reinstalled role loads it from the journal.
    progress("exit6-withdrawal")
    withdrawal = {}
    holder = hold_flow(ctx, ext6, "/evidence/exit6-withdraw-flow.json", 30)
    wait_for(lambda: sum(ok for _, ok in (holder() or {}).get("tcp", [])) >= 3, "IPv6 flow before withdrawal", 30)
    withdrawal["before"] = conntrack6(ext6)
    (report / "signals/exit-withdraw").touch()
    wait_for(lambda: (report / "signals/exit-withdrawn.json").exists(), "withdrawal configured")
    withdrawn = time.time()
    wait_for(lambda: not (status("e").get("exit") or {}).get("installedRole"), "E's role withdrawn", 60)
    withdrawal["after"] = conntrack6(ext6)
    if "mark=" in withdrawal["after"]["stdout"] or withdrawal["after"]["exitCode"]:
        raise RuntimeError("owned IPv6 conntrack survived the withdrawal")
    journal = json.loads(command("e", "cat", "/state/node/underlay-exit.json")["stdout"])
    withdrawal["quarantine"] = [item for item in journal.get("quarantine", []) if ":" in item["tuple"]["source"]]
    if not withdrawal["quarantine"]:
        raise RuntimeError("no retired IPv6 reply tuple was quarantined")
    wait_for(lambda: (holder() or {}).get("done"), "flow across the withdrawal", 60)
    withdrawal["flow"] = holder()
    if any(ok for at, ok in withdrawal["flow"]["tcp"] if at > withdrawn + 2):
        raise RuntimeError("an IPv6 exit flow outlived the withdrawal")
    (report / "signals/exit-restore").touch()
    wait_for(lambda: (report / "signals/exit-restored.json").exists(), "restore configured")
    wait_for(lambda: families_are(ready), "E's role reinstalled", 120)
    nft = json.loads(command("e", "/usr/sbin/nft", "-j", "list", "ruleset")["stdout"])["nftables"]
    loaded = [item["set"] for item in nft if "set" in item and item["set"]["name"] in ("quarantine6", "echo_quarantine6")
              and item["set"]["table"].startswith("meshway_exit_")]
    withdrawal["reloaded"] = loaded
    if not any(item.get("elem") for item in loaded):
        raise RuntimeError("the reinstalled role did not load the IPv6 quarantine from its journal")
    withdrawal["newFlow"] = tcp_echo(ext6)
    if withdrawal["newFlow"]["exitCode"]:
        raise RuntimeError("a new IPv6 flow after the restore failed")
    write(report / "exit6-withdrawal.json", withdrawal)
    stage(6, ["exit6-withdrawal.json"])

    # Down, then the evidence of the whole run: no plaintext IPv6 business
    # packet on A's physical interfaces, A's baseline restored, the foreign
    # sentinel kept.
    progress("exit6-down")
    (report / "signals/exit-down").touch()
    wait_for(lambda: all((report / ("signals/exit-down-" + role)).exists() for role in ("a", "r", "e")), "down")
    down = ctx.snapshot("exit6-down")
    ctx.evidence.no_owned_exit(down)
    for role in ("a", "r", "e"):
        for key in ("addresses", "routes", "rules", "routes6", "rules6"):
            def normalize(record):
                return sorted(json.dumps(item, sort_keys=True) for item in json.loads(record["stdout"]))
            if normalize(ctx.baseline[role][key]) != normalize(down[role][key]):
                raise RuntimeError("down did not restore the owned namespace baseline: " + role + "/" + key)
    after = command("e", "/usr/sbin/nft", "-j", "list", "table", "inet", "meshway_x7b_sentinel")
    if json.loads(after["stdout"]) != json.loads(ctx.sentinel["stdout"]):
        raise RuntimeError("foreign sentinel changed during product cleanup")
    ctx.stop_captures()
    leaks = {}
    for interface in sorted(set(ctx.interfaces["a"].values()) - {"lo"}):
        for path in ctx.capture_files("a", interface):
            record = command("a", "/usr/bin/tcpdump", "-nn", "-r", path, "ip6 and (host " + ext6 + " or host " + far6
                             + " or host " + lan6 + ")", required=False)
            leaks[path] = record["stdout"].count("\n")
    write(report / "exit6-plaintext.json", leaks)
    if any(leaks.values()):
        raise RuntimeError("an IPv6 business packet appeared in plaintext on A's physical interfaces")
    manifest["exit6"] = {"plaintextLeaks": 0}
    ctx.save()


def interface_of(command, role, address):
    names = [link["ifname"] for link in json.loads(command(role, "ip", "-j", "address")["stdout"])
             if any(item.get("local") == address for item in link["addr_info"])]
    if len(names) != 1:
        raise RuntimeError(role + " interface of " + address + " not found")
    return names[0]


def detached_listener(ctx, path, seconds):
    ctx.detached("a", "python3", "/dns-fixture/icmp.py", "listen6", path, str(seconds))
    deadline = time.monotonic() + 10
    while ctx.command("a", "test", "-f", path, required=False)["exitCode"]:
        if time.monotonic() >= deadline:
            raise RuntimeError("A ICMPv6 listener did not start")
        time.sleep(.1)


FLOW = (
    "import json,os,socket,sys,time\n"
    "target,out,duration=sys.argv[1],sys.argv[2],float(sys.argv[3])\n"
    "t=socket.create_connection((target,8080),3); t.settimeout(3)\n"
    "result={'tcp':[],'done':False}\n"
    "def save():\n"
    " open(out+'.tmp','w').write(json.dumps(result)); os.replace(out+'.tmp',out)\n"
    "start=time.monotonic(); i=0\n"
    "while time.monotonic()-start<duration:\n"
    " m=b'exit6-%d'%i; at=time.time()\n"
    " try: t.sendall(m); ok=t.recv(64)==m\n"
    " except OSError: ok=False\n"
    " result['tcp'].append([at,ok]); save(); i+=1; time.sleep(1)\n"
    "result['done']=True; save()\n")


def hold_flow(ctx, target, path, seconds):
    """One long TCP exit flow of A probed every second; returns its reader."""
    ctx.detached("a", "python3", "-c", FLOW, target, path, str(seconds))

    def read():
        record = ctx.command("a", "cat", path, required=False)
        return json.loads(record["stdout"]) if record["exitCode"] == 0 else None
    return read
