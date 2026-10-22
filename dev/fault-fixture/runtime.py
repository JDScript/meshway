"""`fault/throttle`: the external pressures operators applied by hand (OPS
lab: CPU limits, resolved restarts, Control unreachable) as one isolated
row. The daemons own every reaction; this supervisor only applies pressure
from outside them and samples. Gates: `evidence.py`.

Topology: one owned internal bridge with the controller (real Server), A, R,
B and E. A→B goes through R (routing.mesh via R), B's profile sends one
address through its exit E (an egress capture on B, an exit role on E).
Every Node runs systemd-resolved in its own mount namespace
(`dev/dns-fixture/namespace.sh`), the resolved DNS backend and MagicDNS.

Stages, in order, each followed by a 60 s per-thread CPU window of every
meshwayd once the Node under pressure has recovered:
1. cpu-long: B limited to 0.02 CPU for 90 s (`docker update --cpus`).
2. cpu-short: the same for 30 s.
3. resolved-restart: B's resolved restarted twice (a new process and D-Bus
   owner, as `systemctl restart`), 10 s apart.
4. control-outage: the controller's own namespace drops B's Server
   connections both ways for 100 s (nft table in the controller only).
5. exit-audit: E limited to 0.02 CPU for 12 s (the X11 method: its nft reads
   exceed their 2 s budget). The underlay's test-only tool faults (X11／X12)
   exist only in its library tests and cannot be reached from outside the
   daemon process; this external limit replaces them.
The baseline (before stage 1) is one more CPU window.
"""
import importlib.util
import ipaddress
import json
import os
import shutil
import subprocess
import time

from node_lab import ROOT, run
from validation_support import bind_access_options, export_evidence, mount_namespace_options
from verification_sources import fingerprint

_spec = importlib.util.spec_from_file_location("meshway_fault_evidence", ROOT / "dev/fault-fixture/evidence.py")
evidence = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(evidence)

NODES = evidence.NODES
ROLES = ("controller", *NODES)
STAGES = ("cpu-long", "cpu-short", "resolved-restart", "control-outage", "exit-audit")
THROTTLE = 0.02
CPUS = "2"
LONG, SHORT, EXIT_THROTTLE = 90, 30, 12
# Longer than one sync request's budget (90 s, `SYNC_TIMEOUT`): the long poll
# in flight when the drop starts fails inside it.
OUTAGE = 100
IDLE_DELAY, IDLE_WINDOW = 5, 60
# A daemon log beyond this is a loop; the row stops before the disk does.
LOG_LIMIT = 2 * 1024 ** 3
# A target no host answers: only B's capture and E's role need it.
TARGET = "198.51.100.7"
CPU_SAMPLE = ("pid=$(pgrep -xo meshwayd) || exit 3; echo \"pid $pid\"; echo \"clk $(getconf CLK_TCK)\"; "
              "echo \"uptime $(cut -d' ' -f1 /proc/uptime)\"; echo \"proc $(cat /proc/$pid/stat)\"; "
              "for t in /proc/$pid/task/*; do s=$(cat $t/stat 2>/dev/null) && echo \"task ${t##*/} $s\"; done")


def parse_stat(line):
    """(name, utime + stime ticks) of one `/proc/.../stat` line."""
    name = line[line.index("(") + 1:line.rindex(")")]
    rest = line[line.rindex(")") + 2:].split()
    return name, int(rest[11]) + int(rest[12])


def parse_cpu(text):
    sample = {"threads": {}}
    for line in text.splitlines():
        kind, _, value = line.partition(" ")
        if kind == "pid":
            sample["pid"] = int(value)
        elif kind == "clk":
            sample["clkTck"] = int(value)
        elif kind == "uptime":
            sample["uptime"] = float(value)
        elif kind == "proc":
            sample["process"] = parse_stat(value)[1]
        elif kind == "task":
            tid, _, stat = value.partition(" ")
            name, ticks = parse_stat(stat)
            sample["threads"][tid] = {"name": name, "ticks": ticks}
    if not {"pid", "clkTck", "uptime", "process"} <= set(sample):
        raise RuntimeError("incomplete meshwayd CPU sample")
    return sample


def execute(report, artifacts, runtime_os, platform, short, load):
    identity = report.name
    label_key = "io.meshway.runtime-fixture"
    label = label_key + "=" + identity
    image = runtime_os["imageId"]
    containers, networks, addresses = {}, {}, {}
    manifest = {**artifacts, "run": identity, "passed": False, "status": "preparing",
                "scope": "diagnostic-short" if short else "acceptance", "traversal": "fault",
                "dnsBackend": "resolved", "runtimeOsAssets": runtime_os, "loadBeforeRuntime": load,
                "resources": {"containers": containers, "networks": networks}, "cleanup": {}, "processes": {},
                "plannedStages": list(STAGES), "stages": {}}
    metadata = report / "report.json"
    started = time.monotonic()
    failure = None

    def save():
        evidence.write(metadata, manifest)

    def owned(kind, identifier):
        inspect = ["docker", "inspect", identifier] if kind == "container" else ["docker", "network", "inspect", identifier]
        data = json.loads(run(*inspect, capture=True))[0]
        labels = data["Config"]["Labels"] if kind == "container" else data["Labels"]
        if labels.get(label_key) != identity:
            raise RuntimeError("fault fixture ownership changed")
        return data

    def command(role, *arguments, required=True, timeout=20):
        owned("container", containers[role])
        result = subprocess.run(["docker", "exec", containers[role], *arguments],
                                text=True, capture_output=True, timeout=timeout, close_fds=True)
        if required and result.returncode:
            raise RuntimeError(f"fault fixture command failed on {role}: {arguments[0]}: {result.stderr.strip()}")
        return result

    def detached(role, *arguments):
        owned("container", containers[role])
        subprocess.run(["docker", "exec", "-d", containers[role], *arguments], check=True)

    def clock():
        return int(command("controller", "date", "+%s%N").stdout.strip()) / 1e9

    def read_json(role, path):
        try:
            result = command(role, "cat", path, required=False)
            return json.loads(result.stdout) if result.returncode == 0 else None
        except (ValueError, subprocess.TimeoutExpired):
            return None

    def signal(name):
        (report / "signals" / name).write_text("true\n")

    def guard():
        for role in NODES:
            path = report / role / "daemon.stderr"
            if path.exists() and path.stat().st_size > LOG_LIMIT:
                raise RuntimeError("daemon log beyond its bound (a loop): " + role)
        for role in ROLES:
            if not owned("container", containers[role])["State"]["Running"]:
                raise RuntimeError("fault actor stopped: " + role)

    def pause(seconds):
        deadline = time.monotonic() + seconds
        while (left := deadline - time.monotonic()) > 0:
            time.sleep(min(left, 2))
            guard()

    def wait_for(predicate, timeout, interval=.5):
        """Sequencing only (the gates read the samples): False on timeout."""
        deadline = time.monotonic() + timeout
        while True:
            if predicate():
                return True
            if time.monotonic() >= deadline:
                return False
            guard()
            time.sleep(interval)

    def status(role):
        return read_json(role, "/evidence/status.json") or {}

    def running(role, extra=lambda _: True):
        value = status(role)
        expected = manifest.get("serials", {}).get(role)
        return (value.get("phase") == "running" and value.get("dataPlane") == "ready"
                and (value.get("applied") or {}).get("serial") is not None
                and (expected is None or value["applied"]["serial"] == expected) and extra(value))

    def cpu_all():
        found = {}
        for role in NODES:
            try:
                found[role] = parse_cpu(command(role, "/bin/sh", "-c", CPU_SAMPLE).stdout)
            except (RuntimeError, subprocess.TimeoutExpired, ValueError):
                found[role] = None
        return found

    def window(seconds):
        begin = cpu_all()
        pause(seconds)
        end = cpu_all()
        return {role: (begin[role], end[role]) for role in NODES}

    def idle(stage):
        pause(IDLE_DELAY)
        stage["idleCpuStartedAt"] = clock()
        stage["idleCpu"] = window(IDLE_WINDOW)
        stage["endedAt"] = clock()

    def limit(role, cpus):
        owned("container", containers[role])
        run("docker", "update", "--cpus", str(cpus), containers[role], capture=True)

    def throttle(role, seconds, kind, recovered):
        stage = manifest["stages"].setdefault(current[0], {"kind": kind, "node": role, "seconds": seconds})
        stage["injectedAt"] = clock()
        limit(role, THROTTLE)
        try:
            pause(seconds)
        finally:
            limit(role, CPUS)
        stage["restoredAt"] = clock()
        stage["liveRecovered"] = wait_for(recovered, 60)
        idle(stage)

    current = [None]
    save()
    try:
        support = report / "fault-support"
        support.mkdir()
        shutil.copyfile(ROOT / "dev/dns-fixture/namespace.sh", support / "namespace.sh")
        shutil.copyfile(ROOT / "dev/fault-fixture/probe.py", support / "probe.py")
        (support / "os-packages.tsv").write_text("".join(
            package["name"] + "\t" + package["version"] + "\t" + package["architecture"] + "\n"
            for package in runtime_os["packages"]))
        for name in ("signals", "public", "controller", *ROLES):
            (report / name).mkdir(exist_ok=True)
        run("docker", "run", "--rm", "--pull", "never", "--label", label, "--network", "none", "--read-only",
            "--cap-drop", "ALL", *bind_access_options(), "--security-opt", "no-new-privileges", "--entrypoint", "",
            "--tmpfs", "/state:mode=700",
            "--mount", f"type=bind,source={report / 'bin'},target=/fixtures,readonly",
            "--mount", f"type=bind,source={report / 'public'},target=/public",
            "--mount", f"type=bind,source={report / 'controller'},target=/controller",
            image, "/fixtures/meshway-runtime-fixture", "prepare", capture=True)
        networks["wan"] = run("docker", "network", "create", "--internal", "--label", label,
                              f"meshway-runtime-{identity.lower()}-fault", capture=True).strip()
        save()
        inspection = owned("network", networks["wan"])
        subnet = ipaddress.ip_network(inspection["IPAM"]["Config"][0]["Subnet"])
        if (not inspection["Internal"] or inspection["Driver"] != "bridge" or subnet.version != 4
                or subnet.num_addresses < 256 or subnet.overlaps(ipaddress.ip_network("100.64.0.0/10"))):
            raise RuntimeError("fault network must be an internal IPv4 bridge outside the Mesh pool")
        addresses = {role: str(subnet.network_address + 2 + index) for index, role in enumerate(ROLES)}
        # E's default route: an on-link next hop nobody holds, so the exit
        # role has an exit interface and nothing leaves the bridge.
        gateway = str(subnet.network_address + 250)
        manifest["addresses"], manifest["target"], manifest["exitGateway"] = addresses, TARGET, gateway
        evidence.write(report / "public/fault-topology.json", {
            "controller": addresses["controller"], "nodes": {role: addresses[role] for role in NODES},
            "target": TARGET})
        for role in ROLES:
            creation = ["docker", "create", "--pull", "never", "--name", f"meshway-runtime-{identity.lower()}-{role}",
                        "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                        "--security-opt", "no-new-privileges", "--cpus", CPUS, "--memory", "768m",
                        "--pids-limit", "128", "--tmpfs", "/state:rw,nosuid,nodev,mode=700", "--entrypoint", "",
                        "--network", networks["wan"], "--ip", addresses[role],
                        "--sysctl", "net.ipv4.ip_forward=" + ("1" if role == "e" else "0"),
                        "--sysctl", "net.ipv4.conf.all.rp_filter=0", "--sysctl", "net.ipv4.conf.default.rp_filter=0",
                        "--env", "TMPDIR=/state", "--env", "MESHWAY_FAULT_FIXTURE_OWNED=1", *bind_access_options()]
            if role == "controller":
                # Its own namespace's filter drops B's Server connections.
                creation += ["--cap-add", "NET_ADMIN"]
            else:
                creation += ["--cap-add", "NET_ADMIN", "--device", "/dev/net/tun:/dev/net/tun",
                             "--sysctl", "net.ipv6.conf.all.disable_ipv6=0",
                             "--sysctl", "net.ipv6.conf.default.disable_ipv6=0", *mount_namespace_options(),
                             "--tmpfs", "/run:rw,nosuid,nodev,mode=755", "--env", "MESHWAY_DNS_FIXTURE_OWNED=1",
                             "--env", "MESHWAY_DNS_BACKEND=resolved",
                             "--env", "MESHWAY_DNS_UPSTREAM=" + addresses["controller"],
                             "--env", "MESHWAY_DNS_RESTARTS=" + ("2" if role == "b" else "0"),
                             "--env", "DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket"]
                for capability in ("SYS_ADMIN", "CHOWN", "NET_BIND_SERVICE", "NET_RAW", "SETUID", "SETGID",
                                   "SETPCAP", "DAC_OVERRIDE", "SYS_PTRACE", "FOWNER", "KILL"):
                    creation += ["--cap-add", capability]
                creation += ["--mount", f"type=bind,source={support},target=/dns-fixture,readonly",
                             "--mount", f"type=bind,source={support / 'os-packages.tsv'},"
                                        "target=/usr/share/meshway-dns-packages.tsv,readonly"]
                if role == "e":
                    creation += ["--sysctl", "net.netfilter.nf_conntrack_acct=0"]
            mounts = [("bin", "/fixtures", True), (role, "/evidence", False), ("signals", "/signals", False),
                      ("public", "/public", True)]
            if role == "controller":
                mounts.append(("controller", "/controller", True))
            for source, target, readonly in mounts:
                creation += ["--mount", f"type=bind,source={report / source},target={target}" + (",readonly" if readonly else "")]
            creation += [image, *(() if role == "controller" else ("/bin/sh", "/dns-fixture/namespace.sh")),
                         "/fixtures/meshway-runtime-fixture", "--log-format", "json", "--transport", "native-udp",
                         "--traversal", "fault", "--netmap-validity", "1800s", "fault-actor", "--role", role]
            containers[role] = run(*creation, capture=True).strip()
            manifest["resources"].setdefault("commands", {})[role] = creation
            save()
            run("docker", "start", containers[role], capture=True)
        deadline = time.monotonic() + 60
        for role in ROLES:
            while command(role, "test", "-f", "/evidence/ready", required=False).returncode:
                if time.monotonic() >= deadline or not owned("container", containers[role])["State"]["Running"]:
                    raise RuntimeError("fault actor readiness failed: " + role)
                time.sleep(.2)
        command("e", "ip", "route", "replace", "default", "via", gateway, "dev", "eth0")
        manifest["status"] = "enrolling"
        save()
        signal("fault-start")
        configured = None
        deadline = time.monotonic() + 240
        while configured is None:
            if time.monotonic() >= deadline:
                raise RuntimeError("fault configuration timed out")
            guard()
            configured = read_json("controller", "/signals/fault-configured.json")
            time.sleep(.5)
        manifest["configured"] = configured
        if not wait_for(lambda: all(running(role) for role in NODES) and running("e", evidence.exit_ready), 180):
            raise RuntimeError("Nodes did not run with E's exit role ready")
        manifest["serials"] = {role: status(role)["applied"]["serial"] for role in NODES}
        mesh = configured["mesh"]
        manifest["status"] = "baseline"
        save()
        stop = "/signals/fault-traffic-stop"
        detached("b", "python3", "/dns-fixture/probe.py", "serve", mesh["b"], "/evidence/probe", stop)
        if not wait_for(lambda: command("b", "test", "-f", "/evidence/probe/serving", required=False).returncode == 0, 20):
            raise RuntimeError("B's echo did not start")
        detached("a", "python3", "/dns-fixture/probe.py", "client", mesh["b"], "/evidence/probe", stop)
        if not wait_for(lambda: command("a", "grep", "-q", "^e ", "/evidence/probe/client.log",
                                        required=False).returncode == 0, 30):
            raise RuntimeError("A→B flow through R not answered")
        baseline = {"startedAt": clock()}
        manifest["baseline"] = baseline
        pause(10)
        baseline["idleCpu"] = window(IDLE_WINDOW)
        save()
        for name in STAGES:
            current[0] = name
            manifest["status"] = name
            save()
            if name == "cpu-long":
                throttle("b", LONG, "throttle", lambda: running("b"))
            elif name == "cpu-short":
                throttle("b", SHORT, "throttle", lambda: running("b"))
            elif name == "resolved-restart":
                stage = manifest["stages"].setdefault(name, {"kind": "resolved", "node": "b", "restarts": []})
                stage["injectedAt"] = clock()
                begin = cpu_all()
                for number in (1, 2):
                    restart = {"requestedAt": clock()}
                    stage["restarts"].append(restart)
                    signal(f"fault-resolved-restart-b-{number}")
                    path = f"/evidence/resolved-restart-{number}.json"
                    wait_for(lambda: read_json("b", path) is not None, 30, .2)
                    restart["result"] = read_json("b", path)
                    restart["liveRecovered"] = wait_for(lambda: running("b"), 30)
                    pause(10)
                end = cpu_all()
                stage["faultCpu"] = {role: (begin[role], end[role]) for role in NODES}
                stage["restoredAt"] = stage["restarts"][-1]["requestedAt"]
                idle(stage)
            elif name == "control-outage":
                stage = manifest["stages"].setdefault(name, {"kind": "outage", "node": "b", "seconds": OUTAGE})
                b = addresses["b"]
                rules = ("table inet fault_fixture {\n"
                         " chain input { type filter hook input priority 0; policy accept;"
                         f" ip saddr {b} tcp dport 8443 counter drop; }}\n"
                         " chain output { type filter hook output priority 0; policy accept;"
                         f" ip daddr {b} tcp sport 8443 counter drop; }}\n}}\n")
                subprocess.run(["docker", "exec", "-i", containers["controller"], "nft", "-f", "-"], input=rules,
                               text=True, check=True, capture_output=True, timeout=20)
                stage["injectedAt"] = clock()
                try:
                    pause(10)
                    stage["faultCpu"] = window(OUTAGE - 20)
                    pause(max(0.0, stage["injectedAt"] + OUTAGE - clock()))
                    listed = json.loads(command("controller", "nft", "-j", "list", "table", "inet",
                                                "fault_fixture").stdout)
                    counters = [item["rule"] for item in listed["nftables"] if "rule" in item]
                    stage["dropped"] = {rule["chain"]: next(
                        (part["counter"]["packets"] for part in rule["expr"] if "counter" in part), 0)
                        for rule in counters}
                    stage["dropped"] = stage["dropped"] if any(stage["dropped"].values()) else None
                finally:
                    command("controller", "nft", "delete", "table", "inet", "fault_fixture")
                stage["restoredAt"] = clock()
                stage["liveRecovered"] = wait_for(lambda: (lambda value: value.get("lastError") is None
                                                           and value.get("sync") == "connected")(status("b")), 120)
                idle(stage)
            else:
                throttle("e", EXIT_THROTTLE, "exit", lambda: running("e", evidence.exit_ready))
            save()
        manifest["endedAt"] = clock()
        manifest["status"] = "finishing"
        save()
    except BaseException as error:
        failure = error
        manifest["failure"] = f"{type(error).__name__}: {error}"
    finally:
        errors = []
        signal("fault-traffic-stop")
        signal("fault-down")
        # Nodes go down while the Server still answers.
        settle = time.monotonic() + 60
        while time.monotonic() < settle and not all(
                (report / "signals" / f"fault-down-{role}").exists() for role in NODES if role in containers):
            time.sleep(.2)
        signal("fault-stop")
        for role, identifier in containers.items():
            try:
                owned("container", identifier)
                run("docker", "wait", identifier, capture=True, timeout=90)
                state = owned("container", identifier)["State"]
                manifest["processes"][role] = {key: state[key] for key in ("ExitCode", "OOMKilled", "Running")}
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
        for name, identifier in networks.items():
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
                manifest["results"] = evidence.analyse(report, manifest)
            except Exception as error:
                errors.append(f"analysis: {type(error).__name__}: {error}")
        if fingerprint(ROOT) != artifacts["sourceDigest"]:
            errors.append("source changed during fault fixture run")
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
        raise RuntimeError(manifest["status"] + ": inspect private fault report")
