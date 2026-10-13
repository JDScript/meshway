"""Real exit/off process acceptance; product routes and NAT belong to meshwayd."""
import ipaddress
import importlib.util
from datetime import datetime
import json
import shutil
import subprocess
import time
import uuid

from node_lab import ROOT, run, sha256
from validation_support import bind_access_options, export_evidence, mount_namespace_options
from verification_sources import fingerprint


ROLES = ("controller", "a", "r", "e", "external", "lan", "resolver", "router")
# Separate control links prevent the control bridge becoming an A-E shortcut.
# `split` carries only A's own OS DNS upstream, which is also its useWithExit
# split resolver; E's only OS DNS upstream is the external authority. `far`
# is a second public-classified link behind a router on `external`: the
# external service's far address is reached E → router → far, whose router
# interface is narrower (FAR_MTU) than the path before it.
LINKS = {"ca": ("controller", "a"), "cr": ("controller", "r"),
         "ce": ("controller", "e"), "ar": ("a", "r"), "re": ("r", "e"),
         "external": ("e", "external", "router"), "lan": ("e", "lan"), "split": ("a", "resolver"),
         "far": ("router", "external")}
PUBLIC_LINKS = ("external", "far")
FAR_MTU = 1280


NETMAP_CONTENT = "import json; print(json.dumps(json.load(open('/state/node/identity/netmap.json'))['netmap']))"

# Direct ranges of A's split-resolver rule set (tools/meshway-runtime-fixture
# controller/exit.rs): a country-IP-set scale.
DIRECT_RANGES = 7000


def managed(plan, address):
    """Whether a Node's installed capture plan sends `address` to its exit."""
    return any(ipaddress.ip_address(start) <= address <= ipaddress.ip_address(end)
               for start, end in plan["managedIpv4"])

def write(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def run_icmp_phase(command, detached, evidence, addresses, subnets, source):
    """Stage 12: ICMP errors about A's live exit flows return to A, from E
    itself and from a router beyond it; errors naming no flow of A do not.
    PMTU discovery through the narrower far link converges."""
    far, router = addresses["far"]["external"], addresses["external"]["router"]
    exit_address = addresses["external"]["e"]
    result = {"far": far, "router": router, "farMtu": FAR_MTU}
    narrow = [link["ifname"] for link in json.loads(command("router", "ip", "-j", "address")["stdout"])
              if any(item.get("local") == addresses["far"]["router"] for item in link["addr_info"])]
    if len(narrow) != 1:
        raise RuntimeError("router far interface not found")
    result["narrow"] = command("router", "ip", "link", "set", "dev", narrow[0], "mtu", str(FAR_MTU))
    e_addresses = {item["local"] for link in json.loads(command("e", "ip", "-j", "address")["stdout"])
                   for item in link["addr_info"] if item["family"] == "inet"}
    result["route"] = command("e", "ip", "route", "add", subnets["far"], "via", router)
    try:
        def received():
            record = command("a", "cat", "/evidence/icmp-received.json", required=False)
            return json.loads(record["stdout"])["errors"] if record["exitCode"] == 0 else None

        detached("a", "python3", "/dns-fixture/icmp.py", "listen", "/evidence/icmp-received.json", "120")
        deadline = time.monotonic() + 10
        while received() is None:
            if time.monotonic() >= deadline:
                raise RuntimeError("A ICMP listener did not start")
            time.sleep(.1)
        # Hop 1 is E's own time exceeded, hop 2 the router's, hop 3 the target.
        trace = command("a", "traceroute", "-I", "-n", "-q", "1", "-w", "2", "-m", "5", far, required=False)
        hops = {}
        for line in trace["stdout"].splitlines()[1:]:
            fields = line.split()
            if len(fields) >= 2 and fields[0].isdigit():
                hops[int(fields[0])] = fields[1]
        result["traceroute"] = {"record": trace, "hops": hops, "exitAddresses": sorted(e_addresses)}
        if hops.get(1) not in e_addresses or hops.get(2) != router or hops.get(3) != far:
            raise RuntimeError("traceroute through the exit did not show E, the router and the target")
        # A TCP echo larger than the far link converges through fragmentation needed.
        result["tcp"] = command("a", "timeout", "14", "python3", "/dns-fixture/icmp.py", "tcp", far, "8080", "65536",
                                required=False)
        result["routeCache"] = command("a", "ip", "route", "get", far)
        result["pingTooLarge"] = command("a", "ping", "-n", "-M", "do", "-s", "1300", "-c", "2", "-W", "2", far,
                                         required=False)
        result["pingFits"] = command("a", "ping", "-n", "-M", "do", "-s", str(FAR_MTU - 28), "-c", "2", "-W", "2", far,
                                     required=False)
        fragmentation = [entry for entry in received() if (entry["type"], entry["code"]) == (3, 4)
                         and entry["source"] == router and entry["mtu"] == FAR_MTU]
        result["fragmentationNeeded"] = fragmentation
        if (result["tcp"]["exitCode"] or f"mtu {FAR_MTU}" not in result["routeCache"]["stdout"]
                or result["pingTooLarge"]["exitCode"] == 0 or result["pingFits"]["exitCode"] or not fragmentation):
            raise RuntimeError("PMTU discovery through the exit did not converge")
        # Errors injected beyond the exit about A's live UDP flow (post-NAT
        # quote): delivered within the per-flow budget. One naming no flow: none.
        # The flow targets the service's external address: its unbound UDP
        # replies leave from the address of the link they are routed on.
        service = addresses["external"]["external"]
        result["udpTarget"] = service
        detached("a", "python3", "/dns-fixture/icmp.py", "udp", service, "8080", "/evidence/icmp-udp.json", "20")
        deadline = time.monotonic() + 10
        while True:
            record = command("a", "cat", "/evidence/icmp-udp.json", required=False)
            flow = json.loads(record["stdout"]) if record["exitCode"] == 0 else {}
            if flow.get("answered"):
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("A live UDP flow through the exit did not start")
            time.sleep(.2)
        ct = command("e", "/usr/sbin/conntrack", "-L", "-f", "ipv4", "-p", "udp", "-d", service, "-o", "extended")
        nat = evidence.nat_tuple(ct["stdout"], "udp", flow["local"], service + ":8080", exit_address)
        port = int(nat["reply"]["dport"])
        local = int(flow["local"].rsplit(":", 1)[1])
        before = len(received())
        result["forged"] = command("router", "python3", "/dns-fixture/icmp.py", "inject", router, exit_address, "5",
                                   exit_address, str(port + 1), service, "8080")
        result["injected"] = command("router", "python3", "/dns-fixture/icmp.py", "inject", router, exit_address, "30",
                                     exit_address, str(port), service, "8080")
        time.sleep(2)
        arrivals = [entry for entry in received()[before:] if entry["type"] == 11 and entry["source"] == router]
        related = [entry for entry in arrivals if entry.get("quoted", {}).get("source") == source
                   and entry["quoted"]["head"][:8] == f"{local:04x}{8080:04x}"]
        result["arrivals"] = arrivals
        result["related"] = len(related)
        if len(arrivals) != len(related) or not 1 <= len(related) <= 10:
            raise RuntimeError("injected ICMP errors were not limited to A's live flow and its budget")
    finally:
        result["routeRemoved"] = command("e", "ip", "route", "del", subnets["far"], "via", router, required=False)
    return result


def execute(report, artifacts, runtime_os, platform, short, load):
    identity = report.name
    label_key = "io.meshway.runtime-fixture"
    label = label_key + "=" + identity
    image = runtime_os["imageId"]
    containers, networks, addresses, subnets, captures = {}, {}, {}, {}, []
    manifest = {**artifacts, "run": identity, "passed": False, "status": "preparing",
                "scope": "diagnostic-short" if short else "acceptance",
                "traversal": "exit", "dnsBackend": "off", "runtimeOsAssets": runtime_os,
                "loadBeforeRuntime": load, "resources": {"containers": containers, "networks": networks},
                "productStatus": "running", "cleanup": {}, "processes": {},
                "stages": {str(n): {"status": "notRun"} for n in range(1, 13)}}
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
            raise RuntimeError("exit fixture ownership changed")
        return data

    def command(role, *arguments, required=True, timeout=15):
        owned("container", containers[role])
        started_at = time.time_ns()
        result = subprocess.run(["docker", "exec", containers[role], *arguments],
                                text=True, capture_output=True, timeout=timeout, close_fds=True)
        record = {"command": list(arguments), "exitCode": result.returncode,
                  "stdout": result.stdout, "stderr": result.stderr,
                  "startedAtUnixNs": started_at, "finishedAtUnixNs": time.time_ns()}
        if required and result.returncode:
            raise RuntimeError("exit fixture command failed: " + arguments[0])
        return record

    def snapshot(stage):
        data = {}
        for role in ROLES:
            data[role] = {"namespace": command(role, "readlink", "/proc/self/ns/net"),
                          "addresses": command(role, "ip", "-j", "address"),
                           "routes": command(role, "ip", "-j", "route", "show", "table", "all"),
                           "rules": command(role, "ip", "-j", "rule"),
                           "routes6": command(role, "ip", "-6", "-j", "route", "show", "table", "all"),
                           "rules6": command(role, "ip", "-6", "-j", "rule")}
            if role in ("a", "r", "e"):
                # Private diagnostic copy of the cached signed netmap content
                # only: no identity keys; applied comes from IPC, never this file.
                data[role]["netmap"] = command(role, "python3", "-c", NETMAP_CONTENT, required=False)
                data[role]["egressJournal"] = command(role, "cat", "/state/node/underlay-egress.json", required=False)
                data[role]["nft"] = command(role, "/usr/sbin/nft", "-j", "list", "ruleset", required=False)
                data[role]["underlayJournal"] = command(role, "cat", "/state/node/underlay.json", required=False)
                data[role]["exitJournal"] = command(role, "cat", "/state/node/underlay-exit.json", required=False)
                data[role]["tunCounters"] = command(role, "ip", "-j", "-s", "link")
                data[role]["tunReceived"] = command(role, "python3", "-c",
                    "import pathlib,json; print(json.dumps({p.parent.name:int((p.parent/'statistics/rx_packets').read_text()) for p in pathlib.Path('/sys/class/net').glob('*/tun_flags')}))")
        data["controller"]["nodeStates"] = {role: command("controller", "cat", f"/evidence/state-{role}.json", required=False)
                                               for role in ("a", "r", "e")}
        data["e"]["nftCounters"] = command("e", "/usr/sbin/nft", "-j", "list", "ruleset", required=False)
        data["e"]["forwarding"] = command("e", "cat", "/proc/sys/net/ipv4/ip_forward")
        data["e"]["conntrack"] = {
            link: command("e", "/usr/sbin/conntrack", "-L", "-f", "ipv4", "-d", addresses[link][link],
                          "-o", "extended", required=False) for link in ("external", "lan")}
        write(report / (stage + ".json"), data)
        if data["e"]["nftCounters"]["exitCode"]:
            raise RuntimeError("E nft/forwarding precondition unavailable")
        # Preserve an empty table as observed emptiness, not a fabricated NAT tuple.
        if any(item["exitCode"] != 0 for item in data["e"]["conntrack"].values()):
            raise RuntimeError("filtered conntrack read failed")
        return data

    def stop_captures():
        errors = []
        remaining = []
        for role, interface, process, log in captures:
            try:
                if process.poll() is None:
                    command(role, "/bin/sh", "-ec", f"kill -INT $(cat /evidence/{interface}.pid)")
                else:
                    errors.append("capture ended before the complete lifecycle window: " + role + "/" + interface)
                code = process.wait(timeout=15)
                if code:
                    raise RuntimeError("tcpdump failed or did not join")
            except Exception as error:
                errors.append(str(error))
                remaining.append((role, interface, process, log))
            finally:
                log.close()
        captures[:] = remaining
        if errors:
            raise RuntimeError("; ".join(errors))

    save()
    try:
        (report / "exit-support").mkdir()
        shutil.copyfile(ROOT / "dev/dns-fixture/namespace.sh", report / "exit-support/namespace.sh")
        shutil.copyfile(ROOT / "dev/exit-fixture/forwarding.sh", report / "exit-support/forwarding.sh")
        shutil.copyfile(ROOT / "dev/exit-fixture/lease.py", report / "exit-support/lease.py")
        shutil.copyfile(ROOT / "dev/exit-fixture/icmp.py", report / "exit-support/icmp.py")
        (report / "exit-support/os-packages.tsv").write_text("".join(
            package["name"] + "\t" + package["version"] + "\t" + package["architecture"] + "\n"
            for package in runtime_os["packages"]))
        preparation = ["docker", "run", "--rm", "--pull", "never", "--label", label,
                       "--network", "none", "--read-only", "--cap-drop", "ALL", *bind_access_options(),
                       "--security-opt", "no-new-privileges", "--entrypoint", "", "--tmpfs", "/state:mode=700",
                       "--mount", f"type=bind,source={report / 'bin'},target=/fixtures,readonly",
                       "--mount", f"type=bind,source={report / 'public'},target=/public",
                       "--mount", f"type=bind,source={report / 'controller'},target=/controller",
                       image, "/fixtures/meshway-runtime-fixture", "prepare"]
        run(*preparation, capture=True)
        manifest["preparationCommand"] = preparation
        spec = importlib.util.spec_from_file_location("meshway_exit_components", ROOT / "dev/exit-fixture/components.py")
        components = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(components)
        manifest["componentChecks"] = components.execute(report, image, platform, identity)
        spec = importlib.util.spec_from_file_location("meshway_exit_evidence", ROOT / "dev/exit-fixture/evidence.py")
        evidence = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(evidence)
        save()
        for role in ROLES:
            (report / role).mkdir(exist_ok=True)
        for link, roles in LINKS.items():
            # Public classification, isolated by an internal bridge with only E
            # and the service attached. Never route this address onto the host WAN.
            options = []
            if link in PUBLIC_LINKS:
                octet = next(value for value in (uuid.uuid4().int % 256 for _ in range(64))
                             if f"11.77.{value}.0/29" not in subnets.values())
                options = ["--subnet", f"11.77.{octet}.0/29"]
            identifier = run("docker", "network", "create", "--internal", "--label", label, *options,
                             f"meshway-runtime-{identity.lower()}-exit-{link}", capture=True).strip()
            networks[link] = identifier
            save()
            inspection = owned("network", identifier)
            if not inspection["Internal"] or inspection["Driver"] != "bridge":
                raise RuntimeError("exit link must be an internal bridge")
            subnet = ipaddress.ip_network(inspection["IPAM"]["Config"][0]["Subnet"])
            if subnet.version != 4 or subnet.num_addresses < 8:
                raise RuntimeError("exit link requires IPv4 /29 or larger")
            if link not in PUBLIC_LINKS and not any(subnet.subnet_of(ipaddress.ip_network(private)) for private in
                                              ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")):
                raise RuntimeError("exit underlay/LAN must be RFC1918")
            addresses[link] = {role: str(subnet.network_address + 2 + index) for index, role in enumerate(roles)}
            subnets[link] = str(subnet)
            manifest["resources"].setdefault("networkInspections", {})[link] = inspection
        manifest["addresses"] = addresses
        manifest["subnets"] = subnets
        manifest["externalClassification"] = "11.77/16: public-classified isolated address; permit scope must be Internet"
        lan_subnet = manifest["resources"]["networkInspections"]["lan"]["IPAM"]["Config"][0]["Subnet"]
        write(report / "public/exit-topology.json", {"addresses": addresses, "lanCidr": lan_subnet})
        save()
        for role in ROLES:
            links = [link for link, roles in LINKS.items() if role in roles]
            first = links[0]
            creation = ["docker", "create", "--pull", "never", "--name", f"meshway-runtime-{identity.lower()}-{role}",
                        "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                        "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "768m", "--pids-limit", "128",
                        "--tmpfs", "/state:rw,nosuid,nodev,mode=700", "--entrypoint", "",
                        "--network", networks[first], "--ip", addresses[first][role],
                        "--sysctl", "net.ipv4.ip_forward=" + ("1" if role == "router" else "0"),
                        "--sysctl", "net.ipv4.conf.all.rp_filter=0", "--sysctl", "net.ipv4.conf.default.rp_filter=0",
                        "--env", "TMPDIR=/state", "--env", "MESHWAY_EXIT_FIXTURE_OWNED=1", *bind_access_options()]
            if role in ("external", "resolver"):
                creation += ["--cap-add", "NET_BIND_SERVICE"]
            if role == "router":
                creation += ["--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW",
                             "--mount", f"type=bind,source={report / 'exit-support'},target=/dns-fixture,readonly"]
            if role in ("a", "r", "e"):
                upstream = {"a": addresses["split"]["resolver"], "r": addresses["cr"]["controller"],
                            "e": addresses["external"]["external"]}[role]
                creation += ["--cap-add", "NET_ADMIN", "--device", "/dev/net/tun:/dev/net/tun"]
                creation += mount_namespace_options()
                creation += ["--tmpfs", "/run:rw,nosuid,nodev,mode=755", "--env", "MESHWAY_DNS_FIXTURE_OWNED=1",
                             "--env", "MESHWAY_DNS_BACKEND=resolved", "--env", "MESHWAY_DNS_UPSTREAM=" + upstream,
                             "--env", "DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket"]
                for capability in ("SYS_ADMIN", "CHOWN", "NET_BIND_SERVICE", "NET_RAW", "SETUID", "SETGID", "SETPCAP", "DAC_OVERRIDE", "SYS_PTRACE", "FOWNER", "KILL"):
                    creation += ["--cap-add", capability]
                creation += ["--mount", f"type=bind,source={report / 'exit-support'},target=/dns-fixture,readonly",
                             "--mount", f"type=bind,source={report / 'exit-support/os-packages.tsv'},target=/usr/share/meshway-dns-packages.tsv,readonly"]
            if role == "e":
                creation += ["--sysctl", "net.netfilter.nf_conntrack_acct=0"]
            mounts = [("bin", "/fixtures", True), (role, "/evidence", False), ("signals", "/signals", False), ("public", "/public", True)]
            if role == "controller": mounts.append(("controller", "/controller", True))
            for source, target, readonly in mounts:
                creation += ["--mount", f"type=bind,source={report / source},target={target}" + (",readonly" if readonly else "")]
            creation += [image]
            if role in ("a", "r", "e"): creation += ["/bin/sh", "/dns-fixture/namespace.sh"]
            creation += ["/fixtures/meshway-runtime-fixture", *(["--short"] if short else []), "--log-format", "json", "--transport", "native-udp", "--traversal", "exit",
                         "exit-actor", "--role", role]
            containers[role] = run(*creation, capture=True).strip()
            manifest["resources"].setdefault("commands", {})[role] = creation
            save()
            for link in links[1:]:
                run("docker", "network", "connect", "--ip", addresses[link][role], networks[link], containers[role], capture=True)
            run("docker", "start", containers[role], capture=True)
            inspection = owned("container", containers[role])
            if {value["NetworkID"] for value in inspection["NetworkSettings"]["Networks"].values()} != {networks[link] for link in links}:
                raise RuntimeError("unexpected exit actor attachment")
            manifest["resources"].setdefault("actorInspections", {})[role] = inspection
        deadline = time.monotonic() + 30
        for role in ROLES:
            while command(role, "test", "-f", "/evidence/ready", required=False)["exitCode"]:
                if time.monotonic() >= deadline or not owned("container", containers[role])["State"]["Running"]:
                    raise RuntimeError("exit actor readiness failed")
                time.sleep(0.1)
        manifest["status"] = "collecting-infrastructure"
        save()
        interfaces = {}
        for role in ("a", "r", "e"):
            links = json.loads(command(role, "ip", "-j", "address")["stdout"])
            interfaces[role] = {address["local"]: link["ifname"] for link in links for address in link["addr_info"]
                                if address["family"] == "inet"}
            for interface in sorted(set(interfaces[role].values()) - {"lo"}):
                if not interface.isalnum():
                    raise RuntimeError("unexpected Docker interface name")
                log = (report / role / (interface + "-tcpdump.log")).open("w")
                capture = subprocess.Popen(["docker", "exec", containers[role], "/bin/sh", "-ec",
                    f"echo $$ > /evidence/{interface}.pid; exec /usr/bin/tcpdump -Z root -U -n -i {interface} -c 65536 -s 256 -w /evidence/{interface}.pcap"],
                    stdout=log, stderr=subprocess.STDOUT, close_fds=True)
                captures.append((role, interface, capture, log))
        # tcpdump emits its pcap header once capture setup has completed.
        for role, interface, process, _ in captures:
            deadline = time.monotonic() + 10
            while command(role, "test", "-s", f"/evidence/{interface}.pcap", required=False)["exitCode"]:
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("exit packet capture readiness failed")
                time.sleep(0.1)
        command("e", "/usr/sbin/nft", "add", "table", "inet", "meshway_x7b_sentinel")
        sentinel = command("e", "/usr/sbin/nft", "-j", "list", "table", "inet", "meshway_x7b_sentinel")
        write(report / "foreign-sentinel-before.json", sentinel)
        baseline = snapshot("baseline")
        (report / "signals/exit-start").touch()

        def wait_for(predicate, description, timeout=90):
            deadline = time.monotonic() + timeout
            while not predicate():
                if time.monotonic() >= deadline:
                    raise RuntimeError("exit stage timed out: " + description)
                for actor in ROLES:
                    if not owned("container", containers[actor])["State"]["Running"]:
                        raise RuntimeError("exit actor stopped during " + description + ": " + actor)
                time.sleep(.25)

        def status(role):
            result = command(role, "/fixtures/meshway", "--socket", "/state/node/control.sock", "--json", "status", required=False)
            if result["exitCode"]: return {}
            value = json.loads(result["stdout"])
            write(report / role / "observed-status.json", value)
            return value

        def running(value):
            # The signed netmap lease is the only data-plane authority; the
            # instance only bounds State/receipts and is not consulted here.
            return (value.get("phase") == "running" and value.get("dataPlane") == "ready"
                    and value.get("applied") is not None and value["applied"] == value.get("saved")
                    and (value.get("netmap") or {}).get("netmap") == value["applied"])

        observations = {}
        # A's ordinary names are answered only through E's OS resolver; the
        # external authority logs E as their only asker and the resolver
        # (A's own OS upstream, answering everything) never sees them.
        exit_zone = "exit-dns.test"
        dns_tag = identity.lower()
        dns_asked = []
        lookup_program = ("import json,socket,sys\n"
                          "family={'4':socket.AF_INET,'any':socket.AF_UNSPEC}[sys.argv[2]]\n"
                          "try:\n"
                          " found=socket.getaddrinfo(sys.argv[1],None,family,socket.SOCK_DGRAM)\n"
                          " print(json.dumps({'addresses':sorted({a[4][0] for a in found})}))\n"
                          "except socket.gaierror as error:\n"
                          " print(json.dumps({'error':error.errno}))\n")

        def lookup(name, family="4"):
            record = command("a", "timeout", "12", "python3", "-c", lookup_program, name, family, required=False)
            try:
                value = json.loads(record["stdout"])
            except ValueError:
                value = {"error": "no result", "exitCode": record["exitCode"]}
            dns_asked.append(name)
            return {"name": name, "family": family, "value": value, "record": record}

        def resolves(name, expected, family="4", timeout=45):
            attempts = []
            deadline = time.monotonic() + timeout
            while True:
                attempts.append(lookup(name, family))
                if attempts[-1]["value"].get("addresses") == sorted(expected):
                    return attempts
                if time.monotonic() >= deadline:
                    write(report / "dns-failure.json", attempts)
                    raise RuntimeError("system DNS did not return the expected answer: " + name)
                time.sleep(.5)

        def fails(name):
            result = lookup(name)
            if "addresses" in result["value"]:
                write(report / "dns-failure.json", result)
                raise RuntimeError("system DNS answered where it must fail: " + name)
            return result

        def dns_queries(role):
            record = command(role, "cat", "/evidence/dns-queries.json", required=False)
            return json.loads(record["stdout"]) if record["exitCode"] == 0 else []

        def no_source_fallback():
            asked = {query["name"].rstrip(".") for query in dns_queries("resolver")}
            leaked = sorted(name for name in set(dns_asked) if name.endswith(exit_zone) and name in asked)
            if leaked:
                raise RuntimeError("source fell back to its own OS DNS upstream: " + ", ".join(leaked))

        def retired():
            readback = {"e": {
                "nftCounters": command("e", "/usr/sbin/nft", "-j", "list", "ruleset", required=False),
                "conntrack": {link: command("e", "/usr/sbin/conntrack", "-L", "-f", "ipv4", "-d", addresses[link][link], "-o", "extended", required=False)
                              for link in ("external", "lan")},
            }}
            write(report / "retirement-readback.json", readback)
            try:
                evidence.no_owned_exit(readback)
            except RuntimeError:
                return False
            return True

        wait_for(lambda: (report / "signals/exit-configured.json").exists(), "configuration")
        manifest["status"] = "prerequisite"
        save()
        def unavailable():
            value = status("e")
            capability = (value.get("exit") or {}).get("capability") or {}
            server = command("controller", "cat", "/evidence/state-e.json", required=False)
            if server["exitCode"]: return False
            states = json.loads(server["stdout"])["nodes"]
            node = (value.get("identity") or {}).get("nodeId")
            if node not in states: return False
            state = states[node]
            reported = ((state.get("report") or {}).get("report") or {}).get("exitCapability")
            matched = (capability.get("status") == {"status": "unavailable", "reason": "ipForwardDisabled"}
                    and capability.get("installedFamilies") == [] and (value.get("exit") or {}).get("installedRole") is None
                    # An unavailable exit role is a reported fact: the netmap is applied.
                    and running(value) and reported == capability and state["freshness"] == "current")
            if matched:
                observations["unavailable"] = value
                write(report / "e/ip-forward-disabled-server-state.json", json.loads(server["stdout"]))
            return matched
        wait_for(unavailable, "ip_forward=0 fact")
        snapshot("ip-forward-disabled")
        write(report / "e/ip-forward-disabled-status.json", observations["unavailable"])
        manifest["stages"]["2"] = {"status": "passed", "evidence": ["ip-forward-disabled.json", "e/ip-forward-disabled-status.json"]}
        write(report / "forwarding-restored.json", command("e", "/bin/sh", "/dns-fixture/forwarding.sh", "1"))
        if command("e", "cat", "/proc/sys/net/ipv4/ip_forward")["stdout"].strip() != "1":
            raise RuntimeError("fixture forwarding change did not survive namespace readback")
        # No restart and no new netmap: the periodic capability probe sees the
        # restored prerequisite and the same applied content installs the role.
        manifest["status"] = "role-installation"
        save()
        def installed():
            value = status("e")
            capability = (value.get("exit") or {}).get("capability") or {}
            matched = (capability.get("status") == {"status": "ready"}
                    and capability.get("instanceId") == (value.get("instance") or {}).get("instanceId")
                    and capability.get("supportedFamilies") == ["ipv4"]
                    and capability.get("installedFamilies") == ["ipv4"]
                    and capability.get("netmap") == value.get("applied")
                    and bool((value.get("exit") or {}).get("installedRole"))
                    and running(value))
            if matched: observations["installed"] = value
            return matched
        wait_for(installed, "role installed/applied")
        snapshot("installed")
        write(report / "e/installed-status.json", observations["installed"])
        current = observations["installed"]
        acquired = json.loads(command("controller", "cat", "/evidence/acquire-" + current["instance"]["instanceId"] + ".json")["stdout"])
        write(report / "e/acquisition.json", acquired)
        if acquired["capabilities"].get("feature:exit-ipv4") != {"kind": "feature", "version": 1}:
            raise RuntimeError("E did not acquire the executable IPv4 exit capability v1")
        manifest["stages"]["1"] = {"status": "passed", "evidence": ["e/installed-status.json"]}
        manifest["stages"]["3"] = {"status": "passed", "evidence": ["installed.json", "e/installed-status.json"]}
        wait_for(lambda: all(running(status(role)) for role in ("a", "r")), "A/R running with a netmap lease")
        snapshot("capture-before-admission")
        # Actual netmap/kernel readback is retained before the first application
        # packet. Egress default capture and the relay's role remain distinct.
        before = json.loads((report / "capture-before-admission.json").read_text())
        plans = {role: json.loads(before[role]["netmap"]["stdout"])["netmap"] for role in ("a", "r", "e")}
        write(report / "compiled-plans.json", plans)
        # A computes its partition from the netmap rules and the held
        # artifacts; the installed boundary is its own journal and nft sets.
        capture_plan = json.loads(before["a"]["egressJournal"]["stdout"])["plan"]
        a_status = status("a")
        summary = (a_status.get("netmap") or {}).get("capture") or {}
        a_sets = {item["set"]["name"]: item["set"].get("elem", [])
                  for item in json.loads(before["a"]["nft"]["stdout"])["nftables"]
                  if "set" in item and item["set"]["table"].startswith("meshway_egress_")}
        split_address = ipaddress.ip_address(addresses["split"]["resolver"])
        if (not capture_plan.get("defaultCapture") or plans["r"]["exitRole"] is not None
                or plans["e"]["exitRole"] is None):
            raise RuntimeError("A default capture/R non-exit/E exit role mismatch")
        if (summary.get("unavailable") is not None or summary.get("managedIpv4") != len(capture_plan["managedIpv4"])
                or len(capture_plan["managedIpv4"]) < DIRECT_RANGES or len(a_sets.get("managed4", [])) != summary["managedIpv4"]
                or managed(capture_plan, split_address)):
            raise RuntimeError("A's installed partition does not hold the 7,000 direct ranges")
        write(report / "capture-partition.json", {"summary": summary, "nftManaged4": len(a_sets["managed4"]),
                                                  "journalManaged4": len(capture_plan["managedIpv4"])})
        def capture_installed():
            capture_route = command("a", "ip", "-j", "route", "get", addresses["external"]["external"], required=False)
            write(report / "capture-route.json", capture_route)
            if capture_route["exitCode"]: return False
            tun_names = json.loads(before["a"]["tunReceived"]["stdout"])
            return any(route.get("dev") in tun_names for route in json.loads(capture_route["stdout"]))
        wait_for(capture_installed, "default capture readback")
        bypass = {}
        for name, address, port in (("controller", addresses["ca"]["controller"], 8443), ("peer", addresses["ar"]["r"], 35000)):
            bypass[name] = command("a", "python3", "-c", "import socket; socket.create_connection(('" + address + "'," + str(port) + "),2)", required=False)
            if bypass[name]["exitCode"] == 0: raise RuntimeError("ordinary application borrowed the protected " + name + " bypass")
        bypass["ipv6"] = command("a", "ip", "-6", "-j", "route", "get", "2001:4860:4860::8888", required=False)
        if bypass["ipv6"]["exitCode"] == 0 or capture_plan["sinkIpv6"] != [["::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"]]:
            raise RuntimeError("source exit IPv6 is not negatively captured")
        write(report / "capture-bypass.json", bypass)
        manifest["stages"]["4"] = {"status": "passed", "evidence": ["capture-before-admission.json", "compiled-plans.json"]}
        manifest["status"] = "dns"
        save()
        # Single-exit DNS: the source's plan carries E's resolver service on
        # the full A→R→E path; E serves exactly that source.
        node_ids = [plans[role]["nodeId"] for role in ("a", "r", "e")]
        a_dns, e_dns = plans["a"]["dns"], plans["e"]["dns"]
        if (a_dns["egress"].get("type") != "viaExit" or a_dns["egress"]["service"]["path"]["nodes"] != node_ids
                or e_dns["egress"].get("type") != "serveExitResolver"
                or [service["source"] for service in e_dns["egress"]["services"]] != [node_ids[0]]
                or a_dns["resolvers"] != {"split": "direct"}):
            raise RuntimeError("compiled single-exit DNS plans mismatch")
        own = plans["a"]["self"]["addresses"]
        magic = [name for name, value in a_dns["meshHosts"].items() if value == own]
        if len(magic) != 1:
            raise RuntimeError("A's own MagicDNS name missing")
        big = ["198.51.100." + str(i) for i in range(1, 91)]
        dns = {"plans": {"a": a_dns, "e": e_dns}}
        ordinary = "www-" + dns_tag + "." + exit_zone
        dual = "dual-" + dns_tag + "." + exit_zone
        split = "host-" + dns_tag + ".split.test"
        absent = "absent-" + dns_tag + "." + magic[0].split(".", 1)[1].rstrip(".")
        dns["ordinary"] = resolves(ordinary, ["198.51.100.10"])
        # A real AAAA answer is returned as is; IPv6 traffic stays sunk (stage 4).
        dns["dualStack"] = resolves(dual, ["198.51.100.10", "2001:db8::10"], "any")
        # 90 A records exceed the 1232-byte UDP limit of the Mesh leg: the
        # complete answer can only arrive over the separately permitted TCP.
        dns["truncatedToTcp"] = resolves("big." + exit_zone, big)
        dns["magicDns"] = resolves(magic[0].rstrip("."), [own["ipv4"]])
        dns["magicDnsUnknown"] = fails(absent)
        dns["useWithExitSplit"] = resolves(split, ["198.51.100.53"])
        external, resolver = dns_queries("external"), dns_queries("resolver")
        dns["externalQueries"], dns["resolverQueries"] = external, resolver
        for name in (ordinary, dual, "big." + exit_zone):
            peers = {query["peer"].rsplit(":", 1)[0] for query in external if query["name"].rstrip(".") == name}
            if peers != {addresses["external"]["e"]}:
                raise RuntimeError("ordinary name was not resolved by E's OS resolver: " + name)
        split_peers = {query["peer"].rsplit(":", 1)[0] for query in resolver if query["name"].rstrip(".") == split}
        if split_peers != {addresses["split"]["a"]} or any(query["name"].rstrip(".") == split for query in external):
            raise RuntimeError("useWithExit split did not use its own direct resolver")
        for name in (magic[0].rstrip("."), absent):
            if any(query["name"].rstrip(".") == name for query in external + resolver):
                raise RuntimeError("MagicDNS name left the source: " + name)
        no_source_fallback()
        # E's resolver unreachable: A's query fails and never reaches A's own
        # live OS upstream, which would have answered it.
        dns["block"] = command("e", "ip", "route", "add", "unreachable", addresses["external"]["external"] + "/32")
        try:
            dns["blocked"] = fails("blocked-" + dns_tag + "." + exit_zone)
        finally:
            dns["restore"] = command("e", "ip", "route", "del", "unreachable", addresses["external"]["external"] + "/32")
        dns["unblocked"] = resolves("unblocked-" + dns_tag + "." + exit_zone, ["198.51.100.10"])
        no_source_fallback()
        write(report / "dns-exit.json", dns)

        # Domain exit (X6): a name rule sends names under the name zone through
        # E via R, everything else of A is direct. E's served answer completes
        # one association task; A holds the answer until the signed overlay is
        # applied, then connects by name through E.
        manifest["status"] = "domain-exit"
        save()
        named = {}
        target = addresses["external"]["external"]

        def visits_from(peer):
            record = command("external", "cat", "/evidence/access.json", required=False)
            values = json.loads(record["stdout"]) if record["exitCode"] == 0 else []
            return [v for v in values if v["protocol"] == "tcp" and v["peer"].rsplit(":", 1)[0] == peer]

        def accepted_reports():
            record = command("controller", "cat", "/evidence/association-reports.json", required=False)
            return json.loads(record["stdout"]) if record["exitCode"] == 0 else 0

        def by_name(name):
            return command("a", "python3", "-c",
                           "import socket,sys; s=socket.create_connection((sys.argv[1],8080),3); s.settimeout(3); "
                           "s.sendall(b'exit-name'); assert s.recv(64)==b'exit-name'", name, required=False)

        def applied_revision(role, revision):
            value = status(role)
            return running(value) and value["applied"]["revision"] == revision

        def stage_config(signal, done):
            (report / "signals" / signal).touch()
            wait_for(lambda: (report / "signals" / done).exists(), signal)
            revision = json.loads(command("controller", "cat", "/signals/" + done)["stdout"])
            wait_for(lambda: all(applied_revision(role, revision) for role in ("a", "r", "e")), signal + " applied")
            return revision

        named["configured"] = stage_config("exit-name-configure", "exit-name-configured.json")
        # A's installed capture (its egress journal), computed by A itself.
        def a_capture(stage):
            return json.loads(snapshot(stage)["a"]["egressJournal"]["stdout"])["plan"]
        plan = a_capture("domain-exit-configured")
        if plan["defaultCapture"] or managed(plan, ipaddress.ip_address(target)):
            raise RuntimeError("name rule must own no address before an association")
        e_peer = addresses["external"]["e"]
        before = len(visits_from(e_peer))
        # Not through a name: the address follows A's IP rules (direct), and A
        # has no direct route to the external service, so nothing reaches it.
        named["directBefore"] = by_name(target)
        if named["directBefore"]["exitCode"] == 0 or len(visits_from(e_peer)) != before:
            raise RuntimeError("an address without its name was not left to A's IP rules")
        reports = accepted_reports()
        name = "svc-" + dns_tag + "." + exit_zone.replace("exit-dns.test", "name.exit-dns.test")
        started = time.monotonic()
        # The first answer is held until its association applies, then released:
        # one lookup, no SERVFAIL retry.
        named["firstLookup"] = lookup(name)
        named["firstLookupSeconds"] = time.monotonic() - started
        if named["firstLookup"]["value"].get("addresses") != [target]:
            write(report / "dns-failure.json", named["firstLookup"])
            raise RuntimeError("first lookup of a name-rule name did not return its answer")
        named["first"] = by_name(name)
        if named["first"]["exitCode"] or len(visits_from(e_peer)) != before + 1:
            raise RuntimeError("first named connection did not leave through E")
        if any(v["peer"].rsplit(":", 1)[0] != e_peer for v in json.loads(command("external", "cat", "/evidence/access.json")["stdout"])):
            raise RuntimeError("external service saw a peer other than E")
        named["reportsAfterFirst"] = accepted_reports() - reports
        if named["reportsAfterFirst"] < 1:
            raise RuntimeError("no association task completion for the first lookup")
        named["status"] = status("a")
        # The association put exactly the answered address into A's capture.
        if not managed(a_capture("domain-exit-associated"), ipaddress.ip_address(target)):
            raise RuntimeError("associated address is not in A's installed capture")
        named["second"] = by_name(name)
        named["reportsAfterSecond"] = accepted_reports() - reports
        if named["second"]["exitCode"] or named["reportsAfterSecond"] != named["reportsAfterFirst"] or len(visits_from(e_peer)) != before + 2:
            raise RuntimeError("second named connection needed another Server round trip or failed")
        # Address-set semantics: while the association lives, the address
        # itself is decided by the name rule, with or without the name.
        named["directAssociated"] = by_name(target)
        if named["directAssociated"]["exitCode"] or len(visits_from(e_peer)) != before + 3:
            raise RuntimeError("associated address did not follow its name rule")
        # An established flow outlives its association: A keeps the address
        # captured for it, so its packets keep leaving through E (A has no
        # direct path to the external service), while a new connection to
        # the bare address takes the post-association IP decision (direct,
        # dropped). Once the flow ends the address leaves A's capture.
        held_program = (
            "import json,os,socket,sys,time\n"
            "name,duration=sys.argv[1],float(sys.argv[2])\n"
            "address=socket.getaddrinfo(name,8080,socket.AF_INET,socket.SOCK_STREAM)[0][4]\n"
            "t=socket.create_connection(address,3); t.settimeout(3)\n"
            "u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); u.connect(address); u.settimeout(3)\n"
            "result={'address':address[0],'tcp':[],'udp':[],'done':False}\n"
            "def save():\n"
            " open('/evidence/held-flow.tmp','w').write(json.dumps(result)); os.replace('/evidence/held-flow.tmp','/evidence/held-flow.json')\n"
            "def tcp(m):\n"
            " t.sendall(m); return t.recv(64)==m\n"
            "def udp(m):\n"
            " u.send(m); return u.recv(64)==m\n"
            "start=time.monotonic(); i=0\n"
            "while time.monotonic()-start<duration:\n"
            " m=b'held-%d'%i; at=round(time.monotonic()-start,1)\n"
            " for kind,probe in (('tcp',tcp),('udp',udp)):\n"
            "  try: ok=probe(m)\n"
            "  except OSError: ok=False\n"
            "  result[kind].append([at,ok])\n"
            " save(); i+=1; time.sleep(2)\n"
            "t.close(); u.close(); result['done']=True; save()\n")
        owned("container", containers["a"])
        subprocess.run(["docker", "exec", "-d", containers["a"], "python3", "-c", held_program, name, "100"],
                       check=True, timeout=15, close_fds=True)
        def held_flow():
            record = command("a", "cat", "/evidence/held-flow.json", required=False)
            return json.loads(record["stdout"]) if record["exitCode"] == 0 else None
        wait_for(lambda: (held_flow() or {}).get("tcp"), "held flow started", 30)
        # The association lives at most its 60 s TTL after that lookup.
        wait_for(lambda: len((held_flow() or {}).get("tcp", [])) >= 1 and held_flow()["tcp"][-1][0] >= 75,
                 "held flow past its association", 120)
        named["heldAddressAfterExpiry"] = by_name(target)
        if named["heldAddressAfterExpiry"]["exitCode"] == 0:
            raise RuntimeError("the bare address still followed an ended association")
        if not managed(a_capture("domain-exit-held"), ipaddress.ip_address(target)):
            raise RuntimeError("A released the address of a live exit flow")
        wait_for(lambda: (held_flow() or {}).get("done"), "held flow finished", 60)
        held = held_flow()
        named["heldFlow"] = held
        write(report / "held-flow.json", held)
        if not all(ok for _, ok in held["tcp"]) or sum(not ok for _, ok in held["udp"]) > 1:
            raise RuntimeError("an established exit flow failed after its association ended")
        if any(v["peer"].rsplit(":", 1)[0] != e_peer for v in json.loads(command("external", "cat", "/evidence/access.json")["stdout"])):
            raise RuntimeError("external service saw a peer other than E")
        wait_for(lambda: not managed(a_capture("domain-exit-released"), ipaddress.ip_address(target)),
                 "address released after its flows ended", 180)
        named["revoked"] = stage_config("exit-name-revoke", "exit-name-revoked.json")
        named["afterRevoke"] = by_name(name)
        if named["afterRevoke"]["exitCode"] == 0 or len(visits_from(e_peer)) != before + 4:
            raise RuntimeError("named exit outlived its Access revocation")
        named["restored"] = stage_config("exit-name-restore", "exit-name-restored.json")
        write(report / "domain-exit.json", named)
        manifest["stages"]["10"] = {"status": "passed", "evidence": ["domain-exit.json", "domain-exit-configured.json", "held-flow.json",
                                                                    "external/access.json", "controller/association-reports.json"]}

        # Profile switch (C3): A selects another allowed profile (same rules,
        # another selection) through the real CLI; the Server commits it, A applies the netmap carrying it
        # before the command returns, and the switch ends A's exit flows of
        # the previous selection while a new flow uses the new profile.
        # `meshway exit off` then returns to Mesh only.
        manifest["status"] = "profile-switch"
        save()
        switch = {}
        target = addresses["external"]["external"]
        e_node = plans["e"]["nodeId"]

        def exit_cli(*arguments, as_json=True):
            record = command("a", "/fixtures/meshway", "--socket", "/state/node/control.sock",
                             *(["--json"] if as_json else []), "exit", *arguments, required=False, timeout=45)
            if record["exitCode"]:
                write(report / "a" / "exit-failure.json", record)
                raise RuntimeError("meshway exit failed: " + " ".join(arguments))
            return json.loads(record["stdout"]) if as_json else record["stdout"]

        def connect(address):
            return command("a", "python3", "-c",
                           "import socket,sys; s=socket.create_connection((sys.argv[1],8080),3); s.settimeout(3); "
                           "s.sendall(b'exit-profile'); assert s.recv(64)==b'exit-profile'", address, required=False)

        def applied_profile(value, profile):
            local = value.get("local") or {}
            return value.get("applied") is True and local.get("appliedProfile") == profile

        switch["before"] = exit_cli()
        if (switch["before"]["appliedProfile"] != "exit"
                or [p["name"] for p in switch["before"]["profiles"]] != ["exit", "exit-alt", "mesh", "global:" + e_node]):
            raise RuntimeError("A's netmap does not carry the allowed profiles")
        (report / "a" / "exit-view.txt").write_text(exit_cli(as_json=False))
        flow_program = (
            "import json,os,socket,sys,time\n"
            "address,duration=(sys.argv[1],8080),float(sys.argv[2])\n"
            "t=socket.create_connection(address,3); t.settimeout(3)\n"
            "u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); u.connect(address); u.settimeout(3)\n"
            "result={'tcp':[],'udp':[],'done':False}\n"
            "def save():\n"
            " open('/evidence/switch-flow.tmp','w').write(json.dumps(result)); os.replace('/evidence/switch-flow.tmp','/evidence/switch-flow.json')\n"
            "def tcp(m):\n"
            " t.sendall(m); return t.recv(64)==m\n"
            "def udp(m):\n"
            " u.send(m); return u.recv(64)==m\n"
            "start=time.monotonic(); i=0\n"
            "while time.monotonic()-start<duration:\n"
            " m=b'switch-%d'%i\n"
            " for kind,probe in (('tcp',tcp),('udp',udp)):\n"
            "  at=time.time()\n"
            "  try: ok=probe(m)\n"
            "  except OSError: ok=False\n"
            "  result[kind].append([at,ok])\n"
            " save(); i+=1; time.sleep(1)\n"
            "result['done']=True; save()\n")
        owned("container", containers["a"])
        subprocess.run(["docker", "exec", "-d", containers["a"], "python3", "-c", flow_program, target, "40"],
                       check=True, timeout=15, close_fds=True)

        def switch_flow():
            record = command("a", "cat", "/evidence/switch-flow.json", required=False)
            return json.loads(record["stdout"]) if record["exitCode"] == 0 else None
        wait_for(lambda: sum(ok for _, ok in (switch_flow() or {}).get("tcp", [])) >= 3, "exit flow before the switch", 30)
        visits = len(visits_from(e_peer))
        switch["requested"] = time.time()
        switch["alternative"] = exit_cli("exit-alt")
        switch["switched"] = time.time()
        if not applied_profile(switch["alternative"], "exit-alt"):
            raise RuntimeError("the switch did not return after A applied it")
        if switch["alternative"]["local"]["version"] != switch["alternative"]["committed"]["version"]:
            raise RuntimeError("A applied another selection than the Server committed")
        # A new connection follows the new selection through E.
        switch["newFlow"] = connect(target)
        if switch["newFlow"]["exitCode"] or len(visits_from(e_peer)) <= visits:
            raise RuntimeError("a new flow did not leave through E after the switch")
        wait_for(lambda: (switch_flow() or {}).get("done"), "flow across the switch", 60)
        flow = switch_flow()
        switch["flow"] = flow
        for kind in ("tcp", "udp"):
            # A probe in flight while the switch commits may go either way.
            before = [ok for at, ok in flow[kind] if at < switch["requested"] - 1]
            after = [ok for at, ok in flow[kind] if at > switch["switched"] + 1]
            if not all(before) or not after or any(after):
                raise RuntimeError("the switch did not end the previous selection's " + kind + " flow")
        plan = a_capture("profile-alternative")
        if not plan["defaultCapture"] or not managed(plan, ipaddress.ip_address(target)):
            raise RuntimeError("the alternative profile does not capture external traffic")
        visits = len(visits_from(e_peer))
        switch["off"] = exit_cli("off")
        if not applied_profile(switch["off"], "mesh"):
            raise RuntimeError("meshway exit off did not apply mesh")
        plan = a_capture("profile-mesh")
        if plan["defaultCapture"] or plan["managedIpv4"]:
            raise RuntimeError("mesh profile still captures external traffic")
        switch["meshFlow"] = connect(target)
        if switch["meshFlow"]["exitCode"] == 0 or len(visits_from(e_peer)) != visits:
            raise RuntimeError("an external flow left through E under mesh")
        (report / "a" / "exit-off.txt").write_text(exit_cli(as_json=False))
        switch["restored"] = exit_cli("exit")
        if not applied_profile(switch["restored"], "exit"):
            raise RuntimeError("the exit profile was not restored")
        wait_for(lambda: running(status("a")), "A running after the restored profile")
        write(report / "profile-switch.json", switch)
        manifest["stages"]["11"] = {"status": "passed", "evidence": ["profile-switch.json", "profile-alternative.json", "profile-mesh.json",
                                                                    "a/exit-view.txt", "a/exit-off.txt"]}

        manifest["status"] = "traffic"
        save()
        (report / "signals/exit-traffic").touch()
        wait_for(lambda: command("a", "test", "-f", "/evidence/original-tuples.json", required=False)["exitCode"] == 0, "first NAT UDP/TCP", 100)
        snapshot("first-flow")
        wait_for(lambda: (report / "signals/exit-window-done").exists(), "persistent TCP/UDP/receive-only", 360)
        after = snapshot("after-traffic")
        # Netmap model: no permit documents exist. Each node decides locally on
        # its own projection; the evidence is the projected inputs plus the
        # real packets (A–E share no link, R TUN/pcap carry no plaintext).
        projected = {role: json.loads(after[role]["netmap"]["stdout"])["netmap"] for role in ("a", "r", "e")}
        path = [projected[role]["nodeId"] for role in ("a", "r", "e")]
        if any(json.loads(after["r"]["tunReceived"]["stdout"]).values()):
            raise RuntimeError("relay TUN delivered business plaintext")
        grant = {"src": ["node:" + path[0]], "dst": sorted(["autogroup:internet", lan_subnet])}
        routes = {}
        exit_action = {"type": "exit", "nodes": [path[2]], "via": [path[1]]}
        plan_a = projected["a"]["egress"]
        installed_capture = json.loads(after["a"]["egressJournal"]["stdout"])["plan"]
        actions = [rule["action"] for rule in plan_a["rules"]] + [plan_a["fallback"]]
        if any(action["type"] == "exit" and action != exit_action for action in actions):
            raise RuntimeError("A egress plan names an exit other than E via R")
        for link in ("external", "lan"):
            target = ipaddress.ip_address(addresses[link][link])
            if not managed(installed_capture, target):
                raise RuntimeError("A egress partition does not send " + link + " to E via R")
            routes[link] = {"managed": True, "action": exit_action}
        for role in ("a", "r", "e"):
            rules = [{"src": sorted(rule["src"]), "dst": sorted(rule["dst"])} for rule in projected[role]["projection"]["configuration"]["access"]]
            if rules != [grant]:
                raise RuntimeError("projected external Access differs on " + role)
        # Only autogroup:internet can cover the external target: it is public
        # and outside the single explicit LAN CIDR, the LAN target is inside it.
        external, lan = (ipaddress.ip_address(addresses[link][link]) for link in ("external", "lan"))
        if not external.is_global or external in ipaddress.ip_network(lan_subnet) or lan not in ipaddress.ip_network(lan_subnet):
            raise RuntimeError("Internet/explicit LAN classification mismatch")
        write(report / "original-path.json", {"path": path, "routes": routes, "access": grant,
                                              "exitRole": projected["e"]["exitRole"]})
        tuples = json.loads(command("a", "cat", "/evidence/original-tuples.json")["stdout"])
        proof = []
        for link, udp_local, udp_remote, tcp_local, tcp_remote in tuples:
            ct = after["e"]["conntrack"][link]["stdout"]
            for protocol, local, remote in (("udp", udp_local, udp_remote), ("tcp", tcp_local, tcp_remote)):
                flow = evidence.nat_tuple(ct, protocol, local, remote, addresses[link]["e"])
                if flow["mark"] != json.loads(after["e"]["exitJournal"]["stdout"])["program"]["mark"]:
                    raise RuntimeError("NAT conntrack mark does not belong to the exit journal")
                visits = json.loads(command(link, "cat", "/evidence/access.json")["stdout"])
                if not any(v["protocol"] == protocol and v["peer"] == addresses[link]["e"] + ":" + flow["reply"]["dport"] for v in visits):
                    raise RuntimeError("NAT tuple absent from external service log")
                proof.append(flow)
        write(report / "nat-proof.json", proof)
        def tcp_probe(role, target):
            return command(role, "python3", "-c",
                           "import socket; s=socket.create_connection(('" + target + "',8080),2); s.settimeout(2); "
                           "s.sendall(b'exit-block-control'); assert s.recv(64)==b'exit-block-control'", required=False)

        def warm_probe(role, target):
            attempts = []
            deadline = time.monotonic() + 45
            while True:
                result = tcp_probe(role, target)
                attempts.append(result)
                if result["exitCode"] == 0 or time.monotonic() >= deadline:
                    return {"exitCode": result["exitCode"], "attempts": attempts}
                time.sleep(.1)

        manifest["stages"]["5"] = {"status": "passed", "evidence": ["nat-proof.json", "first-flow.json", "components/driver.log"]}
        manifest["stages"]["6"] = {"status": "passed", "evidence": ["components/driver.log", "components/handoff.log", "a/receive-only-external.json", "a/receive-only-lan.json"]}

        # Withdraw the still-live original sockets before any egress fault can
        # independently terminate them and make this assertion vacuous.
        manifest["status"] = "withdrawal"
        save()
        (report / "signals/exit-withdraw").touch()
        wait_for(lambda: (report / "signals/exit-withdrawn.json").exists(), "role withdrawal")
        withdrawn_revision = json.loads(command("controller", "cat", "/signals/exit-withdrawn.json")["stdout"])
        wait_for(lambda: running(s := status("e")) and s["applied"]["revision"] == withdrawn_revision
                 and (s.get("exit") or {}).get("installedRole") is None, "role drain applied")
        wait_for(retired, "owned role/conntrack retirement readback")
        evidence.no_owned_exit(snapshot("withdrawn"))
        # Role withdrawal ends the source's exit DNS plan with the role.
        wait_for(lambda: ((status("a").get("saved") or {}).get("revision")) == withdrawn_revision, "source withdrawal netmap")
        dns_lifecycle = {"withdrawn": fails("withdrawn-" + dns_tag + "." + exit_zone)}
        (report / "signals/exit-flow-withdraw").touch()
        wait_for(lambda: (report / "signals/exit-traffic-done").exists(), "old socket refusal")
        (report / "signals/exit-restore").touch()
        wait_for(lambda: (report / "signals/exit-restored.json").exists(), "role restoration")
        wait_for(installed, "restored applied")
        dns_lifecycle["restored"] = resolves("restored-" + dns_tag + "." + exit_zone, ["198.51.100.10"], timeout=90)

        controls = {}
        for link in ("external", "lan"):
            target = addresses[link][link]
            controls[link] = {"a": warm_probe("a", target)}
            for role in ("e", "r", "controller"):
                controls[link][role + "Direct"] = tcp_probe(role, target)
            if controls[link]["eDirect"]["exitCode"] or any(controls[link][role + "Direct"]["exitCode"] == 0 for role in ("r", "controller")):
                raise RuntimeError("external service infrastructure is not E-only")
            controls[link]["block"] = command("e", "ip", "route", "add", "unreachable", target + "/32")
            try:
                controls[link]["blocked"] = tcp_probe("a", target)
            finally:
                controls[link]["restore"] = command("e", "ip", "route", "del", "unreachable", target + "/32")
            controls[link]["restored"] = warm_probe("a", target)
            write(report / "business-block-control.json", controls)
            if controls[link]["a"]["exitCode"] or not controls[link]["blocked"]["exitCode"] or controls[link]["restored"]["exitCode"]:
                raise RuntimeError("A business flow did not follow E block/restore")
            echo = command("a", "ping", "-n", "-c", "3", "-W", "2", target, required=False)
            write(report / ("icmp-" + link + ".json"), echo)
            if echo["exitCode"]: raise RuntimeError("real exit ICMP echo failed")
        manifest["status"] = "icmp-errors"
        save()
        def detached(role, *arguments):
            owned("container", containers[role])
            subprocess.run(["docker", "exec", "-d", containers[role], *arguments], check=True, timeout=15,
                           close_fds=True)

        icmp = run_icmp_phase(command, detached, evidence, addresses, subnets, own["ipv4"])
        write(report / "icmp-errors.json", icmp)
        manifest["stages"]["12"] = {"status": "passed", "evidence": ["icmp-errors.json"]}
        # HTTP outage under the netmap model: closing Node HTTP never withdraws
        # the exit. Existing and new flows continue on the signed lease, with
        # no Server contact, until its actual expiry, which fails closed.
        def at(value):
            return datetime.fromisoformat(value).timestamp()

        before_outage = {role: status(role) for role in ("a", "r", "e")}
        write(report / "before-outage-status.json", before_outage)
        probes = {}
        try:
            for link in ("external", "lan"):
                log = (report / "a" / ("lease-" + link + ".log")).open("w")
                probes[link] = (subprocess.Popen(
                    ["docker", "exec", containers["a"], "python3", "/dns-fixture/lease.py", addresses[link][link],
                     "/evidence/lease-" + link + ".ready", "/evidence/lease-" + link + ".json", "400"],
                    stdout=log, stderr=subprocess.STDOUT, close_fds=True), log)
                wait_for(lambda: command("a", "test", "-f", "/evidence/lease-" + link + ".ready", required=False)["exitCode"] == 0
                         or probes[link][0].poll() is not None, "lease probe first echo " + link, 60)
                if probes[link][0].poll() is not None:
                    raise RuntimeError("lease probe ended before the outage: " + link)
            manifest["status"] = "http-outage"
            save()
            (report / "signals/exit-http-close").touch()
            wait_for(lambda: (report / "signals/exit-http-closed.json").exists(), "HTTP closed")
            wait_for(lambda: (report / "signals/exit-http-held").exists(), "real in-flight Node HTTP held")
            closed_at = json.loads(command("controller", "cat", "/signals/exit-http-closed.json")["stdout"]) / 1000
            frozen = {role: status(role) for role in ("a", "r", "e")}
            write(report / "outage-leases.json", {"closedAt": closed_at, "status": frozen})
            if not all(running(value) for value in frozen.values()) or not (frozen["e"].get("exit") or {}).get("installedRole"):
                raise RuntimeError("closing Node HTTP withdrew authority before lease expiry")
            # Within the signed lease the exit resolver serves with no Server.
            dns_lifecycle["offline"] = resolves("offline-" + dns_tag + "." + exit_zone, ["198.51.100.10"], timeout=10)
            # Every admitted request has drained: these leases can no longer be renewed.
            last_contact = max([closed_at] + [at(value["lastSyncAt"]) for value in frozen.values() if value.get("lastSyncAt")])
            expiries = {role: at(value["netmap"]["expiresAt"]) for role, value in frozen.items()}
            first_expiry, last_expiry = min(expiries.values()), max(expiries.values())
            if first_expiry <= last_contact:
                raise RuntimeError("no offline lease window after the last Server contact")
            if last_expiry - time.time() > 150:
                raise RuntimeError("lease exceeds the fixture's held-request budget")
            observed = {"eInstalledAt": None}

            def e_expired():
                value = status("e")
                installed_role = (value.get("exit") or {}).get("installedRole")
                if value.get("applied") is not None and installed_role:
                    observed["eInstalledAt"] = time.time()
                return value.get("applied") is None and value.get("netmap") is None and installed_role is None
            wait_for(e_expired, "actual netmap lease expiry on E", last_expiry - time.time() + 60)
            withdrawn_at = time.time()
            if observed["eInstalledAt"] is None or observed["eInstalledAt"] < expiries["e"] - 3 or withdrawn_at > expiries["e"] + 15:
                raise RuntimeError("E exit was not withdrawn at its own lease expiry")
            leases = {"closedAt": closed_at, "lastContact": last_contact, "expiries": expiries,
                      "eInstalledAt": observed["eInstalledAt"], "eWithdrawnAt": withdrawn_at, "flows": {}}
            for link, (process, log) in probes.items():
                code = process.wait(timeout=60)
                log.close()
                result = json.loads(command("a", "cat", "/evidence/lease-" + link + ".json")["stdout"])
                write(report / ("lease-" + link + ".json"), result)
                persistent = [item["at"] for item in result["rounds"]]
                fresh = [item["at"] for item in result["rounds"] if item["udp"]]
                ended = result["failure"]
                leases["flows"][link] = {"lastPersistent": max(persistent, default=None), "lastFresh": max(fresh, default=None),
                                         "failure": ended}
                if code or ended is None:
                    raise RuntimeError("exit flow outlived the netmap lease: " + link)
                if not any(t > last_contact for t in persistent) or not any(t > last_contact for t in fresh):
                    raise RuntimeError("no existing/new exit flow after the last Server contact: " + link)
                if any(item["udp"] is False and item["at"] < first_expiry - 5 for item in result["rounds"]):
                    raise RuntimeError("new exit flow refused within the lease: " + link)
                if not first_expiry - 3 <= ended["at"] <= first_expiry + 6:
                    raise RuntimeError("exit flow did not end at the first path lease expiry: " + link)
            probes.clear()
            write(report / "lease-window.json", leases)
        finally:
            for process, log in probes.values():
                if process.poll() is None:
                    process.kill()
                process.wait()
                log.close()
        wait_for(lambda: all(status(role).get("applied") is None for role in ("a", "r")), "A/R lease expiry", 60)
        dns_lifecycle["expired"] = fails("expired-" + dns_tag + "." + exit_zone)
        wait_for(retired, "expired role/conntrack retirement readback")
        expired = snapshot("expired")
        evidence.no_owned_exit(expired)
        evidence.negative_boundary(expired["a"]["nft"])
        write(report / "e/expired-status.json", status("e"))
        (report / "signals/exit-http-open").touch()
        wait_for(lambda: (report / "signals/exit-http-opened.json").exists(), "HTTP restored")
        wait_for(installed, "automatic lease recovery")
        recovered = status("e")
        if recovered["identity"] != before_outage["e"]["identity"] or at(recovered["netmap"]["expiresAt"]) <= expiries["e"]:
            raise RuntimeError("lease recovery changed identity or reused the expired lease")
        write(report / "e/recovered-status.json", recovered)
        snapshot("recovered")
        wait_for(lambda: all(running(status(role)) for role in ("a", "r")), "source/relay automatic recovery")
        dns_lifecycle["recovered"] = resolves("recovered-" + dns_tag + "." + exit_zone, ["198.51.100.10"], timeout=60)
        no_source_fallback()
        write(report / "dns-lifecycle.json", dns_lifecycle)
        for link in ("external", "lan"):
            result = warm_probe("a", addresses[link][link])
            write(report / ("recovered-flow-" + link + ".json"), result)
            if result["exitCode"]: raise RuntimeError("new flow after lease recovery failed")
        before_crash = snapshot("before-crash")
        (report / "signals/exit-crash-e").touch()
        wait_for(lambda: (report / "signals/exit-crashed-e").exists(), "SIGKILL joined")
        gap = snapshot("crash-gap")
        (report / "signals/exit-restart-after-crash").touch()
        wait_for(lambda: (report / "signals/exit-crash-restarted-e").exists(), "journal restart")
        wait_for(installed, "same-namespace journal recovery")
        after_crash = snapshot("after-crash")
        if before_crash["e"]["namespace"]["stdout"] != gap["e"]["namespace"]["stdout"] or gap["e"]["namespace"]["stdout"] != after_crash["e"]["namespace"]["stdout"]:
            raise RuntimeError("journal recovery changed network namespace")
        for link in ("external", "lan"):
            old = {json.dumps(flow["original"], sort_keys=True) for flow in evidence.conntrack_flows(before_crash["e"]["conntrack"][link]["stdout"]) if flow["mark"]}
            live = {json.dumps(flow["original"], sort_keys=True) for flow in evidence.conntrack_flows(after_crash["e"]["conntrack"][link]["stdout"]) if flow["mark"]}
            if old & live: raise RuntimeError("old owned conntrack survived same-namespace journal recovery")
        (report / "signals/exit-down").touch()
        wait_for(lambda: all((report / ("signals/exit-down-" + role)).exists() for role in ("a", "r", "e")), "offline down")
        down = snapshot("down")
        evidence.no_owned_exit(down)
        for role in ("a", "r", "e"):
            for key in ("addresses", "routes", "rules", "routes6", "rules6"):
                normalize = lambda record: sorted(json.dumps(item, sort_keys=True) for item in json.loads(record["stdout"]))
                if normalize(baseline[role][key]) != normalize(down[role][key]):
                    raise RuntimeError("offline down did not restore exact owned namespace baseline: " + role + "/" + key)
        after_sentinel = command("e", "/usr/sbin/nft", "-j", "list", "table", "inet", "meshway_x7b_sentinel")
        write(report / "foreign-sentinel-after.json", after_sentinel)
        if json.loads(after_sentinel["stdout"]) != json.loads(sentinel["stdout"]):
            raise RuntimeError("foreign sentinel changed during product cleanup")
        manifest["stages"]["8"] = {"status": "passed", "evidence": ["withdrawn.json", "a/withdrawn-old-flows.json", "lease-window.json", "expired.json", "recovered.json", "down.json", "components/role.log"]}
        stop_captures()
        # Ordinary names cross A's and R's links only inside the protected
        # DNS circuit; E's capture shows its OS resolver asking for them.
        for role in ("a", "r", "e"):
            for interface in sorted(set(interfaces[role].values()) - {"lo"}):
                found = command(role, "python3", "-c", "import pathlib; b=pathlib.Path('/evidence/" + interface + ".pcap').read_bytes().lower(); print(int(b'exit-dns' in b))")["stdout"].strip()
                external_link = role == "e" and interfaces["e"].get(addresses["external"]["e"]) == interface
                if found != ("1" if external_link else "0"):
                    raise RuntimeError("ordinary DNS name capture mismatch on " + role + "/" + interface)
        no_source_fallback()
        manifest["dnsNoSourceFallback"] = {"passed": True, "evidence": "dns-exit.json"}
        manifest["stages"]["9"] = {"status": "passed", "evidence": ["dns-exit.json", "dns-lifecycle.json", "e/*.pcap", "external/dns-queries.json", "resolver/dns-queries.json"]}
        manifest["packetTuples"] = {}
        for role in ("r", "e"):
            manifest["packetTuples"][role] = {}
            for interface in sorted(set(interfaces[role].values()) - {"lo"}):
                record = command(role, "/usr/bin/tcpdump", "-nn", "-tt", "-r", f"/evidence/{interface}.pcap")
                write(report / role / (interface + "-tuples.json"), record)
                manifest["packetTuples"][role][interface] = {"status": "collected", "natProof": None}
                if role == "r":
                    # The pcap includes every physical interface, but never a
                    # source/exit plaintext delivery. Component driver separately
                    # asserts SecureNode::exit_reply and zero relay flows.
                    body = command(role, "python3", "-c", "import pathlib; b=pathlib.Path('/evidence/" + interface + ".pcap').read_bytes(); print(int(b'meshway-exit-encrypted-through-r' in b or bytes([90])*64 in b))")["stdout"]
                    if body.strip() != "0": raise RuntimeError("relay pcap contains business plaintext")
        manifest["stages"]["7"] = {"status": "passed", "evidence": ["components/driver.log", "nat-proof.json", "r/*-tuples.json"]}
        manifest["infrastructure"] = "collected"
        manifest["status"] = "finishing"
    except BaseException as error:
        failure = error
        manifest["failedAt"] = manifest["status"]
        manifest["status"] = "failed"
        manifest["failure"] = str(error)
        if "e" in containers:
            try:
                write(report / "failure-exit-readback.json", {
                    "forwarding": command("e", "cat", "/proc/sys/net/ipv4/ip_forward", required=False),
                    "nft": command("e", "/usr/sbin/nft", "-j", "list", "ruleset", required=False),
                    "conntrack": command("e", "/usr/sbin/conntrack", "-L", "-f", "ipv4", "-o", "extended", required=False),
                    "netmap": command("e", "python3", "-c", NETMAP_CONTENT, required=False),
                })
            except Exception as readback_error:
                manifest["failureReadbackError"] = str(readback_error)
    finally:
        errors = []
        try:
            stop_captures()
        except Exception as error:
            errors.append(str(error))
        (report / "signals/exit-stop").touch()
        for role, identifier in containers.items():
            try:
                owned("container", identifier)
                run("docker", "wait", identifier, capture=True, timeout=20)
                state = owned("container", identifier)["State"]
                manifest["processes"][role] = state
                (report / (role + ".log")).write_text(run("docker", "logs", identifier, capture=True))
                if state["Running"] or state["OOMKilled"] or state["ExitCode"] != 0:
                    errors.append("actor did not finish successfully: " + role)
            except Exception as error:
                errors.append(str(error))
            try:
                owned("container", identifier)
                run("docker", "rm", "-f", identifier, capture=True)
                manifest["cleanup"][role] = True
            except Exception as error:
                errors.append(str(error))
        for link, identifier in reversed(list(networks.items())):
            try:
                owned("network", identifier)
                run("docker", "network", "rm", identifier, capture=True)
                manifest["cleanup"]["network-" + link] = True
            except Exception as error:
                errors.append(str(error))
        for _, _, process, log in captures:
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                errors.append("capture attach client required termination")
            finally:
                log.close()
        try:
            export_evidence(report, image, platform, "runtime", identity)
            manifest["evidenceSha256"] = {str(path.relative_to(report)): sha256(path) for path in report.rglob("*.pcap")}
        except Exception as error:
            errors.append(str(error))
        if fingerprint(ROOT) != artifacts["sourceDigest"]:
            errors.append("source changed during exit fixture run")
        sources = {str(path.relative_to(ROOT)): sha256(path) for path in sorted((ROOT / "dev/exit-fixture").glob("*")) if path.is_file()}
        if sources != artifacts["exitFixtureSources"]:
            errors.append("exit fixture sources changed during run")
        if errors:
            manifest["cleanupErrors"] = errors
            manifest["status"] = "failed"
        if failure is None and not errors and manifest["status"] == "finishing":
            manifest["passed"] = evidence.all_stages_passed(manifest)
            manifest["status"] = "passed" if manifest["passed"] else "incomplete"
        manifest["runtimeElapsedSeconds"] = time.monotonic() - started
        save()
        print(report, flush=True)
    if failure:
        raise failure
    if not manifest["passed"]:
        raise RuntimeError(manifest["status"] + ": inspect private exit report")
