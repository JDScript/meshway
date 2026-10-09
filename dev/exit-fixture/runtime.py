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


ROLES = ("controller", "a", "r", "e", "external", "lan")
# Separate control links prevent the control bridge becoming an A-E shortcut.
LINKS = {"ca": ("controller", "a"), "cr": ("controller", "r"),
         "ce": ("controller", "e"), "ar": ("a", "r"), "re": ("r", "e"),
         "external": ("e", "external"), "lan": ("e", "lan")}


def write(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def execute(report, artifacts, runtime_os, platform, short, load):
    identity = report.name
    label_key = "io.meshway.runtime-fixture"
    label = label_key + "=" + identity
    image = runtime_os["imageId"]
    containers, networks, addresses, captures = {}, {}, {}, []
    manifest = {**artifacts, "run": identity, "passed": False, "status": "preparing",
                "scope": "diagnostic-short" if short else "acceptance",
                "traversal": "exit", "dnsBackend": "off", "runtimeOsAssets": runtime_os,
                "loadBeforeRuntime": load, "resources": {"containers": containers, "networks": networks},
                "productStatus": "running", "cleanup": {}, "processes": {},
                "stages": {str(n): {"status": "notRun"} for n in range(1, 9)}}
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

    def command(role, *arguments, required=True):
        owned("container", containers[role])
        started_at = time.time_ns()
        result = subprocess.run(["docker", "exec", containers[role], *arguments],
                                text=True, capture_output=True, timeout=15, close_fds=True)
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
                # Private diagnostic copy only. Do not export identity keys or
                # infer applied from this persisted (saved) compiled document.
                data[role]["savedExecution"] = command(role, "python3", "-c",
                    "import json; print(json.dumps(json.load(open('/state/node/identity/control.json'))['saved']))", required=False)
                data[role]["underlayJournal"] = command(role, "cat", "/state/node/underlay.json", required=False)
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
        write(report / "exit-support/os-packages.tsv", runtime_os)
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
            options = ["--subnet", f"11.77.{uuid.uuid4().int % 256}.0/29"] if link == "external" else []
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
            if link != "external" and not any(subnet.subnet_of(ipaddress.ip_network(private)) for private in
                                              ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")):
                raise RuntimeError("exit underlay/LAN must be RFC1918")
            addresses[link] = {role: str(subnet.network_address + 2 + index) for index, role in enumerate(roles)}
            manifest["resources"].setdefault("networkInspections", {})[link] = inspection
        manifest["addresses"] = addresses
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
                        "--sysctl", "net.ipv4.ip_forward=0",
                        "--sysctl", "net.ipv4.conf.all.rp_filter=0", "--sysctl", "net.ipv4.conf.default.rp_filter=0",
                        "--env", "TMPDIR=/state", "--env", "MESHWAY_EXIT_FIXTURE_OWNED=1", *bind_access_options()]
            if role in ("a", "r", "e"):
                creation += ["--cap-add", "NET_ADMIN", "--device", "/dev/net/tun:/dev/net/tun"]
                creation += mount_namespace_options()
                creation += ["--tmpfs", "/run:rw,nosuid,nodev,mode=755", "--env", "MESHWAY_DNS_FIXTURE_OWNED=1",
                             "--env", "MESHWAY_DNS_BACKEND=resolved", "--env", "MESHWAY_DNS_UPSTREAM=" + addresses["c" + role]["controller"],
                             "--env", "DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket"]
                for capability in ("SYS_ADMIN", "CHOWN", "NET_BIND_SERVICE", "NET_RAW", "SETUID", "SETGID", "SETPCAP", "DAC_OVERRIDE", "SYS_PTRACE", "FOWNER", "KILL"):
                    creation += ["--cap-add", capability]
                creation += ["--mount", f"type=bind,source={report / 'exit-support'},target=/dns-fixture,readonly",
                             "--mount", f"type=bind,source={report / 'exit-support/os-packages.tsv'},target=/usr/share/meshway-dns-packages.tsv,readonly"]
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
        for role in ("r", "e"):
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
        snapshot("baseline")
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
            return (capability.get("status") == {"status": "unavailable", "reason": "ipForwardDisabled"}
                    and capability.get("installedFamilies") == [] and (value.get("exit") or {}).get("installedRole") is None
                    and value.get("applied") is None and reported == capability and state["freshness"] == "current")
        wait_for(unavailable, "ip_forward=0 fact")
        snapshot("ip-forward-disabled")
        write(report / "e/ip-forward-disabled-status.json", status("e"))
        manifest["stages"]["2"] = {"status": "passed", "evidence": ["ip-forward-disabled.json", "e/ip-forward-disabled-status.json"]}
        write(report / "forwarding-restored.json", command("e", "/bin/sh", "/dns-fixture/forwarding.sh", "1"))
        if command("e", "cat", "/proc/sys/net/ipv4/ip_forward")["stdout"].strip() != "1":
            raise RuntimeError("fixture forwarding change did not survive namespace readback")
        (report / "signals/exit-restart-e").touch()
        wait_for(lambda: (report / "signals/exit-restarted-e").exists(), "same-namespace daemon restart")
        manifest["status"] = "role-installation"
        save()
        def installed():
            value = status("e")
            capability = (value.get("exit") or {}).get("capability") or {}
            return (capability.get("status") == {"status": "ready"}
                    and capability.get("instanceId") == (value.get("instance") or {}).get("instanceId")
                    and capability.get("installedFamilies") == ["ipv4"]
                    and capability.get("target") == value.get("applied")
                    and bool((value.get("exit") or {}).get("installedRole"))
                    and value.get("applied") is not None and value["applied"] == value.get("saved"))
        wait_for(installed, "role installed/applied")
        snapshot("installed")
        write(report / "e/installed-status.json", status("e"))
        current = status("e")
        acquired = json.loads(command("controller", "cat", "/evidence/acquire-" + current["instance"]["instanceId"] + ".json")["stdout"])
        write(report / "e/acquisition.json", acquired)
        if acquired["capabilities"].get("feature:exit-ipv4") != {"kind": "feature", "version": 1}:
            raise RuntimeError("E did not acquire the executable IPv4 exit capability v1")
        manifest["stages"]["1"] = {"status": "passed", "evidence": ["e/installed-status.json"]}
        manifest["stages"]["3"] = {"status": "passed", "evidence": ["installed.json"]}
        wait_for(lambda: all((s := status(role)).get("applied") is not None and s["applied"] == s.get("saved") for role in ("a", "r")), "A/R applied")
        snapshot("capture-before-admission")
        # Actual policy/kernel readback is retained before the first application
        # packet. Egress default capture and the relay's role remain distinct.
        before = json.loads((report / "capture-before-admission.json").read_text())
        a_saved = json.loads(before["a"]["savedExecution"]["stdout"])
        r_saved = json.loads(before["r"]["savedExecution"]["stdout"])
        write(report / "compiled-plans.json", {"a": a_saved, "r": r_saved})
        if not a_saved["configuration"]["egressCapture"]["defaultCapture"] or r_saved["configuration"]["exitRole"] is not None:
            raise RuntimeError("A default capture/R non-exit role mismatch")
        capture_route = command("a", "ip", "-j", "route", "get", addresses["external"]["external"])
        write(report / "capture-route.json", capture_route)
        tun_names = json.loads(before["a"]["tunReceived"]["stdout"])
        if not any(route.get("dev") in tun_names for route in json.loads(capture_route["stdout"])):
            raise RuntimeError("A external route readback does not select the installed TUN")
        manifest["stages"]["4"] = {"status": "passed", "evidence": ["capture-before-admission.json", "compiled-plans.json"]}
        manifest["status"] = "traffic"
        save()
        (report / "signals/exit-traffic").touch()
        wait_for(lambda: command("a", "test", "-f", "/evidence/original-tuples.json", required=False)["exitCode"] == 0, "first NAT UDP/TCP", 100)
        snapshot("first-flow")
        wait_for(lambda: (report / "signals/exit-window-done").exists(), "persistent TCP/UDP/receive-only", 360)
        after = snapshot("after-traffic")
        compiled = {role: json.loads(after[role]["savedExecution"]["stdout"])["configuration"] for role in ("a", "r", "e")}
        path = [compiled[role]["nodeId"] for role in ("a", "r", "e")]
        permits = list(compiled["r"]["permits"].values())
        if not permits or any(p["exit"] is None or p["source"] != path[0] or p["replies"] != "statefulExactReversePath"
                              or any(candidate["nodes"] != path for candidate in p["paths"]) for p in permits):
            raise RuntimeError("relay lacks the original external transit permit/path")
        if any(json.loads(after["r"]["tunReceived"]["stdout"]).values()):
            raise RuntimeError("relay TUN delivered business plaintext")
        write(report / "original-path.json", {"path": path, "permits": list(permits)})
        for link in ("external", "lan"):
            matching = [p for p in permits if p["target"] == {"type": "ip", "address": addresses[link][link]}]
            expected = {"type": "internet"} if link == "external" else {"type": "cidr", "prefix": lan_subnet}
            if not matching or any(p["exit"]["destination"] != expected for p in matching):
                raise RuntimeError("Internet/explicit LAN permit classification mismatch")
        tuples = json.loads(command("a", "cat", "/evidence/original-tuples.json")["stdout"])
        proof = []
        for link, udp_local, udp_remote, tcp_local, tcp_remote in tuples:
            ct = after["e"]["conntrack"][link]["stdout"]
            for protocol, local, remote in (("udp", udp_local, udp_remote), ("tcp", tcp_local, tcp_remote)):
                flow = evidence.nat_tuple(ct, protocol, local, remote, addresses[link]["e"])
                visits = json.loads(command(link, "cat", "/evidence/access.json")["stdout"])
                if not any(v["protocol"] == protocol and v["peer"] == addresses[link]["e"] + ":" + flow["reply"]["dport"] for v in visits):
                    raise RuntimeError("NAT tuple absent from external service log")
                proof.append(flow)
        write(report / "nat-proof.json", proof)
        def tcp_probe(role, target):
            return command(role, "python3", "-c",
                           "import socket; s=socket.create_connection(('" + target + "',8080),2); s.settimeout(2); "
                           "s.sendall(b'exit-block-control'); assert s.recv(64)==b'exit-block-control'", required=False)

        controls = {}
        for link in ("external", "lan"):
            target = addresses[link][link]
            controls[link] = {"a": tcp_probe("a", target)}
            controls[link]["block"] = command("e", "ip", "route", "add", "unreachable", target + "/32")
            try:
                controls[link]["blocked"] = tcp_probe("a", target)
            finally:
                controls[link]["restore"] = command("e", "ip", "route", "del", "unreachable", target + "/32")
            controls[link]["restored"] = tcp_probe("a", target)
            write(report / "business-block-control.json", controls)
            if controls[link]["a"]["exitCode"] or not controls[link]["blocked"]["exitCode"] or controls[link]["restored"]["exitCode"]:
                raise RuntimeError("A business flow did not follow E block/restore")
            echo = command("a", "ping", "-n", "-c", "3", "-W", "2", target, required=False)
            write(report / ("icmp-" + link + ".json"), echo)
            if echo["exitCode"]: raise RuntimeError("real exit ICMP echo failed")
        manifest["stages"]["5"] = {"status": "passed", "evidence": ["nat-proof.json", "first-flow.json", "components/driver.log"]}
        manifest["stages"]["6"] = {"status": "passed", "evidence": ["components/driver.log", "components/handoff.log", "a/receive-only-external.json", "a/receive-only-lan.json"]}

        # Remove the role while the source retains its original TCP/UDP sockets.
        manifest["status"] = "withdrawal"
        save()
        (report / "signals/exit-withdraw").touch()
        wait_for(lambda: (report / "signals/exit-withdrawn.json").exists(), "role withdrawal")
        wait_for(lambda: (status("e").get("exit") or {}).get("installedRole") is None, "role drain")
        evidence.no_owned_exit(snapshot("withdrawn"))
        (report / "signals/exit-flow-withdraw").touch()
        wait_for(lambda: (report / "signals/exit-traffic-done").exists(), "old socket refusal")
        (report / "signals/exit-restore").touch()
        wait_for(lambda: (report / "signals/exit-restored.json").exists(), "role restoration")
        wait_for(installed, "restored applied")
        before_expiry = status("e")
        write(report / "e/before-expiry.json", before_expiry)
        manifest["status"] = "http-expiry"
        save()
        (report / "signals/exit-http-close").touch()
        wait_for(lambda: (report / "signals/exit-http-closed.json").exists(), "HTTP closed")
        actual_deadline = max(datetime.fromisoformat(status(role)["instance"]["expiresAt"]).timestamp() for role in ("a", "r", "e"))
        wait_for(lambda: status("e").get("applied") is None and (status("e").get("exit") or {}).get("installedRole") is None,
                 "actual instance/execution expiry", 180)
        wait_for(lambda: time.time() > actual_deadline, "original instance deadline", 180)
        evidence.no_owned_exit(snapshot("expired"))
        write(report / "e/expired-status.json", status("e"))
        (report / "signals/exit-http-open").touch()
        wait_for(lambda: (report / "signals/exit-http-opened.json").exists(), "HTTP restored")
        wait_for(installed, "automatic instance recovery")
        recovered = status("e")
        if recovered["instance"]["instanceId"] == before_expiry["instance"]["instanceId"] or recovered["identity"] != before_expiry["identity"]:
            raise RuntimeError("instance replacement/identity recovery mismatch")
        write(report / "e/recovered-status.json", recovered)
        snapshot("recovered")
        wait_for(lambda: all((s := status(role)).get("applied") is not None and s["applied"] == s.get("saved") for role in ("a", "r")), "source/relay automatic recovery")
        for link in ("external", "lan"):
            result = tcp_probe("a", addresses[link][link])
            write(report / ("recovered-flow-" + link + ".json"), result)
            if result["exitCode"]: raise RuntimeError("new flow after instance recovery failed")
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
        evidence.no_owned_exit(snapshot("down"))
        after_sentinel = command("e", "/usr/sbin/nft", "-j", "list", "table", "inet", "meshway_x7b_sentinel")
        write(report / "foreign-sentinel-after.json", after_sentinel)
        if json.loads(after_sentinel["stdout"]) != json.loads(sentinel["stdout"]):
            raise RuntimeError("foreign sentinel changed during product cleanup")
        manifest["stages"]["8"] = {"status": "passed", "evidence": ["withdrawn.json", "a/withdrawn-old-flows.json", "expired.json", "recovered.json", "down.json", "components/role.log"]}
        stop_captures()
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
                    "saved": command("e", "python3", "-c", "import json; print(json.dumps(json.load(open('/state/node/identity/control.json'))['saved']))", required=False),
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
