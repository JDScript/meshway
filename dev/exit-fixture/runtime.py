"""Owned exit/off infrastructure; never installs product routes, NAT or permits."""
import ipaddress
import json
import subprocess
import time
import uuid

from node_lab import ROOT, run, sha256
from validation_support import bind_access_options, export_evidence
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
                "productStatus": "pending-x2-x5", "cleanup": {}, "processes": {}}
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
        if data["e"]["nftCounters"]["exitCode"] or data["e"]["forwarding"]["stdout"].strip() != "1":
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
        for role in ROLES:
            (report / role).mkdir(exist_ok=True)
        for link, roles in LINKS.items():
            # TEST-NET-2 is a simulation target, NEVER an Internet-classification pass.
            options = ["--subnet", f"198.51.100.{(uuid.uuid4().int % 32) * 8}/29"] if link == "external" else []
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
        manifest["externalClassification"] = "TEST-NET-2 reserved simulation; not Internet authorization evidence"
        save()
        for role in ROLES:
            links = [link for link, roles in LINKS.items() if role in roles]
            first = links[0]
            creation = ["docker", "create", "--pull", "never", "--name", f"meshway-runtime-{identity.lower()}-{role}",
                        "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                        "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "256m", "--pids-limit", "64",
                        "--tmpfs", "/state:rw,nosuid,nodev,mode=700", "--entrypoint", "",
                        "--network", networks[first], "--ip", addresses[first][role],
                        "--sysctl", "net.ipv4.ip_forward=" + ("1" if role == "e" else "0"),
                        "--env", "MESHWAY_EXIT_FIXTURE_OWNED=1", *bind_access_options()]
            if role in ("a", "r", "e"):
                creation += ["--cap-add", "NET_ADMIN", "--device", "/dev/net/tun:/dev/net/tun"]
            if role in ("r", "e"):
                creation += ["--cap-add", "NET_RAW", "--cap-add", "SETUID", "--cap-add", "SETGID"]
            for source, target, readonly in (("bin", "/fixtures", True), (role, "/evidence", False), ("signals", "/signals", True)):
                creation += ["--mount", f"type=bind,source={report / source},target={target}" + (",readonly" if readonly else "")]
            creation += [image, "/fixtures/meshway-runtime-fixture", "--log-format", "json", "--transport", "native-udp", "--traversal", "exit",
                         "exit-actor", "--role", role]
            containers[role] = run(*creation, capture=True).strip()
            manifest["resources"].setdefault("commands", {})[role] = creation
            save()
            for link in links[1:]:
                run("docker", "network", "connect", "--ip", addresses[link][role], networks[link], containers[role], capture=True)
            inspection = owned("container", containers[role])
            if {value["NetworkID"] for value in inspection["NetworkSettings"]["Networks"].values()} != {networks[link] for link in links}:
                raise RuntimeError("unexpected exit actor attachment")
            manifest["resources"].setdefault("actorInspections", {})[role] = inspection
            run("docker", "start", containers[role], capture=True)
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
        probes = {}
        for link in ("external", "lan"):
            target = addresses[link][link]
            request = ("curl", "--noproxy", "*", "--fail", "--silent", "--show-error", "--connect-timeout", "1", "--max-time", "2", f"http://{target}:8080/")
            probes[link] = {role: command(role, *request, required=False) for role in ("e", "a", "r", "controller")}
            write(report / "infrastructure-probes.json", probes)
            if probes[link]["e"]["exitCode"] or any(probes[link][role]["exitCode"] == 0 for role in ("a", "r", "controller")):
                raise RuntimeError("external/LAN reachability is not E-only")
            interface = interfaces["e"][addresses[link]["e"]]
            probes[link]["block"] = command("e", "ip", "link", "set", "dev", interface, "down")
            try:
                probes[link]["eBlocked"] = command("e", *request, required=False)
            finally:
                probes[link]["restore"] = command("e", "ip", "link", "set", "dev", interface, "up")
            probes[link]["eRestored"] = command("e", *request, required=False)
            write(report / "infrastructure-probes.json", probes)
            if probes[link]["eBlocked"]["exitCode"] == 0 or probes[link]["eRestored"]["exitCode"]:
                raise RuntimeError("E-only infrastructure block/restore control failed")
        write(report / "infrastructure-probes.json", probes)
        snapshot("after-probes")
        stop_captures()
        manifest["packetTuples"] = {}
        for role in ("r", "e"):
            manifest["packetTuples"][role] = {}
            for interface in sorted(set(interfaces[role].values()) - {"lo"}):
                record = command(role, "/usr/bin/tcpdump", "-nn", "-tt", "-r", f"/evidence/{interface}.pcap")
                write(report / role / (interface + "-tuples.json"), record)
                manifest["packetTuples"][role][interface] = {"status": "infrastructure-only", "natProof": None}
        manifest["infrastructure"] = "collected"
        manifest["businessBlockControl"] = "pending-x2-x5"
        manifest["status"] = "unsupported"
    except BaseException as error:
        failure = error
        manifest["status"] = "infrastructure-failed"
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
                if state["Running"] or state["OOMKilled"] or state["ExitCode"] != 1:
                    errors.append("actor did not finish with unsupported: " + role)
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
            if manifest["status"] == "unsupported":
                for role in ROLES:
                    manifest.setdefault("actors", {})[role] = json.loads((report / role / "exit-stages.json").read_text())
                    if "unsupported: exit product stages pending-x2-x5" not in (report / role / "error.txt").read_text():
                        errors.append("missing unsupported actor result: " + role)
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
            manifest["status"] = "infrastructure-failed"
        manifest["runtimeElapsedSeconds"] = time.monotonic() - started
        save()
        print(report, flush=True)
    if failure:
        raise failure
    raise RuntimeError(manifest["status"] + ": exit product stages pending-x2-x5; inspect private report")
