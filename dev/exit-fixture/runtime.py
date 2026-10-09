"""Real exit/off process acceptance; product routes and NAT belong to meshwayd."""
import ipaddress
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
                          "rules": command(role, "ip", "-j", "rule")}
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

    def stop_captures():
        errors = []
        remaining = []
        for role, interface, process, log in captures:
            try:
                if process.poll() is None:
                    command(role, "/bin/sh", "-ec", f"kill -INT $(cat /evidence/{interface}.pid)")
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
                    f"echo $$ > /evidence/{interface}.pid; exec /usr/bin/tcpdump -Z root -U -n -i {interface} -c 4096 -s 256 -w /evidence/{interface}.pcap"],
                    stdout=log, stderr=subprocess.STDOUT, close_fds=True)
                captures.append((role, interface, capture, log))
        # tcpdump emits its pcap header once capture setup has completed.
        for role, interface, process, _ in captures:
            deadline = time.monotonic() + 10
            while command(role, "test", "-s", f"/evidence/{interface}.pcap", required=False)["exitCode"]:
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("exit packet capture readiness failed")
                time.sleep(0.1)
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
            return (capability.get("status") == {"status": "unavailable", "reason": "ipForwardDisabled"}
                    and capability.get("installedFamilies") == [] and (value.get("exit") or {}).get("installedRole") is None)
        wait_for(unavailable, "ip_forward=0 fact")
        snapshot("ip-forward-disabled")
        write(report / "e/ip-forward-disabled-status.json", status("e"))
        manifest["stages"]["2"] = {"status": "passed", "evidence": ["ip-forward-disabled.json", "e/ip-forward-disabled-status.json"]}
        command("e", "sysctl", "-w", "net.ipv4.ip_forward=1")
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
                    and bool((value.get("exit") or {}).get("installedRole"))
                    and value.get("applied") is not None and value["applied"] == value.get("saved"))
        wait_for(installed, "role installed/applied")
        snapshot("installed")
        write(report / "e/installed-status.json", status("e"))
        manifest["stages"]["1"] = {"status": "passed", "evidence": ["e/installed-status.json"]}
        manifest["stages"]["3"] = {"status": "passed", "evidence": ["installed.json"]}
        wait_for(lambda: all((s := status(role)).get("applied") is not None and s["applied"] == s.get("saved") for role in ("a", "r")), "A/R applied")
        snapshot("capture-before-admission")
        manifest["stages"]["4"] = {"status": "observed", "evidence": ["capture-before-admission.json"]}
        manifest["status"] = "traffic"
        save()
        (report / "signals/exit-traffic").touch()
        wait_for(lambda: command("a", "test", "-f", "/evidence/original-tuples.json", required=False)["exitCode"] == 0, "first NAT UDP/TCP", 100)
        snapshot("first-flow")
        wait_for(lambda: (report / "signals/exit-traffic-done").exists(), "persistent TCP/UDP", 360)
        snapshot("after-traffic")
        stop_captures()
        manifest["packetTuples"] = {}
        for role in ("r", "e"):
            manifest["packetTuples"][role] = {}
            for interface in sorted(set(interfaces[role].values()) - {"lo"}):
                record = command(role, "/usr/bin/tcpdump", "-nn", "-tt", "-r", f"/evidence/{interface}.pcap")
                write(report / role / (interface + "-tuples.json"), record)
                manifest["packetTuples"][role][interface] = {"status": "collected", "natProof": None}
        manifest["infrastructure"] = "collected"
        manifest["status"] = "incomplete"
        raise RuntimeError("remaining lifecycle assertions have not run")
    except BaseException as error:
        failure = error
        manifest["failedAt"] = manifest["status"]
        manifest["status"] = "failed"
        manifest["failure"] = str(error)
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
        manifest["runtimeElapsedSeconds"] = time.monotonic() - started
        save()
        print(report, flush=True)
    if failure:
        raise failure
    if not manifest["passed"]:
        raise RuntimeError(manifest["status"] + ": inspect private exit report")
