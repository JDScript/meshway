#!/usr/bin/env python3
"""Isolated real-Agent / kernel-NAT acceptance. Main invokes explicitly under tmux."""

import argparse
from contextlib import contextmanager
import fcntl
import ipaddress
import json
import os
from pathlib import Path
import platform
import re
import signal
import stat
import subprocess
import sys
import tempfile
import time
import uuid

sys.dont_write_bytecode = True
from checks import CheckError, crosscheck, parse_endpoint, require, strict_json, utc_now, validate_report
from kernel import SCENARIOS, router_commands
from wire import endpoint

TEMP = Path("/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode")
LOCK = TEMP / "meshway-performance.lock"
LABEL = "io.meshway.netcheck.run"
ROLE_LABEL = "io.meshway.netcheck.role"
HERE = Path(__file__).resolve().parent
ID = re.compile(r"[0-9a-f]{64}")
IMAGE = re.compile(r"meshway-netcheck:[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}")


class Interrupted(KeyboardInterrupt):
    pass


def interrupt(_signal, _frame):
    raise Interrupted()


@contextmanager
def performance_lock():
    descriptor = os.open(LOCK, os.O_CREAT | os.O_RDWR | getattr(os, "O_NOFOLLOW", 0), 0o600)
    try:
        metadata = os.fstat(descriptor)
        require(stat.S_ISREG(metadata.st_mode) and metadata.st_uid == os.getuid() and metadata.st_mode & 0o077 == 0, "performance lock must be private and owned")
        print("Waiting for shared Meshway performance lock", file=sys.stderr, flush=True)
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield descriptor
    finally:
        os.close(descriptor)


def architecture(value):
    return {"arm64": "arm64", "aarch64": "arm64", "amd64": "amd64", "x86_64": "amd64"}.get(value)


def container_args(run_id, role, name, network, image_id):
    administrator = role in ("client", "router")
    uid = 0 if administrator else 65532
    args = [
        "container", "create", "--name", name,
        "--label", f"{LABEL}={run_id}", "--label", f"{ROLE_LABEL}={role}",
        "--network", network, "--cap-drop", "ALL", "--read-only",
        "--security-opt", "no-new-privileges:true", "--user", f"{uid}:{uid}",
        "--cpus", "1", "--memory", "256m", "--memory-swap", "256m", "--pids-limit", "64",
        "--tmpfs", f"/private:rw,noexec,nosuid,nodev,size=4m,mode=0700,uid={uid},gid={uid}",
        "--log-driver", "none", "--entrypoint", "sleep",
    ]
    if administrator:
        args += ["--cap-add", "NET_ADMIN"]
    if role == "router":
        args += ["--sysctl", "net.ipv4.ip_forward=1"]
    return [*args, image_id, "infinity"]


def counters(raw):
    """Do not persist raw iptables-save output; retain only our named counters."""
    allowed = {
        "netcheck-snat-a", "netcheck-snat-b", "netcheck-forward-a", "netcheck-forward-b",
        "netcheck-return", "netcheck-drop-all", "netcheck-drop-b", "netcheck-canary",
    }
    result = {}
    for line in raw.splitlines():
        match = re.match(r"\[(\d+):(\d+)\] -A (\w+) .*--comment \"?(netcheck-[a-z-]+)\"?(?: |$)", line)
        if match:
            name = match[4]
            require(name in allowed and name not in result, "unexpected router counter")
            result[name] = {"packets": int(match[1]), "bytes": int(match[2])}
    require(result, "router counters unavailable")
    return result


def conntrack_mappings(raw, client, wan, servers, bound_port, samples):
    """Check the original and reply tuples; retain only validated fixture endpoints."""
    mappings = []
    wanted = {sample["server"]: sample["mapped_endpoint"] for sample in samples if sample["outcome"] == "mapped"}
    for line in raw.splitlines():
        pairs = re.findall(r"(?:^| )(src|dst|sport|dport)=([^ ]+)", line)
        if not pairs or pairs[0] != ("src", client):
            continue
        require([key for key, _value in pairs] == ["src", "dst", "sport", "dport"] * 2, "unexpected conntrack tuple shape")
        values = [value for _key, value in pairs]
        original_src, original_dst, source_port, destination_port, reply_src, reply_dst, reply_port, mapped_port = values
        require(original_dst in servers and destination_port == reply_port == "3478" and reply_src == original_dst, "conntrack destination outside fixture")
        require(original_src == client and reply_dst == wan and source_port == str(bound_port), "probe must use its declared single socket through router SNAT")
        server = endpoint(original_dst, 3478)
        require(mapped_port.isdecimal() and 1 <= int(mapped_port) <= 65535, "invalid conntrack mapped port")
        mapped = endpoint(reply_dst, int(mapped_port))
        require(server in wanted and wanted[server] == mapped, "kernel reply tuple differs from Agent observation")
        mappings.append({"server": server, "client_socket": endpoint(client, bound_port), "mapped_endpoint": mapped})
    require(len(mappings) == len(wanted) and {item["server"] for item in mappings} == set(wanted), "successful probe destinations need kernel conntrack evidence")
    return mappings


class Docker:
    def __init__(self, lock_fd):
        self.lock_fd = lock_fd

    def call(self, args, timeout=20, checked=True):
        # Never emit command output/errors; inherited lock survives an interrupted
        # wrapper until the current Docker client has exited.
        result = subprocess.run(["docker", *args], stdin=subprocess.DEVNULL, capture_output=True,
                                text=True, timeout=timeout, pass_fds=(self.lock_fd,))
        if checked:
            require(result.returncode == 0, "Docker command failed (raw diagnostics suppressed)")
        return result

    def text(self, args, timeout=20):
        return self.call(args, timeout).stdout.strip()

    def json(self, args, timeout=20):
        return strict_json(self.text(args, timeout))

    def inspect(self, kind, name):
        result = self.call([kind, "inspect", name], checked=False)
        if result.returncode:
            missing = ("No such container:", "No such network:", "No such object:")
            if any(part in result.stderr for part in missing):
                return None
            raise CheckError("owned-resource inspection failed")
        value = strict_json(result.stdout)
        require(type(value) is list and len(value) == 1 and type(value[0]) is dict, "invalid Docker inspection")
        return value[0]


class Harness:
    def __init__(self, docker, run_id):
        self.docker = docker
        self.run_id = run_id
        self.resources = []
        self.containers = {}
        self.networks = {}
        self.addresses = {}
        self.subnets = {}
        self.stage = "image_preflight"

    def owned(self, inspected, role, kind):
        labels = inspected.get("Config", {}).get("Labels") if kind == "container" else inspected.get("Labels")
        require(type(labels) is dict and labels.get(LABEL) == self.run_id and labels.get(ROLE_LABEL) == role, "resource ownership mismatch")

    def image(self, name):
        require(IMAGE.fullmatch(name) is not None, "use dedicated meshway-netcheck:TAG image")
        metadata = self.docker.json(["image", "inspect", name])
        require(type(metadata) is list and len(metadata) == 1, "image inspection failed")
        image = metadata[0]
        image_id = image.get("Id")
        require(isinstance(image_id, str) and re.fullmatch(r"sha256:[0-9a-f]{64}", image_id), "image must resolve to full immutable ID")
        native = architecture(platform.machine())
        daemon = self.docker.json(["info", "--format", '{"os":{{json .OSType}},"architecture":{{json .Architecture}}}'])
        require(native is not None and image.get("Os") == daemon.get("os") == "linux", "Linux Docker image/daemon required")
        require(image.get("Architecture") == architecture(daemon.get("architecture")) == native, "host, image and daemon must have matching native architecture")
        require(not image.get("Config", {}).get("Volumes"), "image must not create anonymous volumes")
        return {"requested_tag": name, "immutable_id": image_id, "architecture": native}

    def create_network(self, role):
        name = f"meshway-netcheck-{self.run_id}-{role}"
        resource = {"kind": "network", "role": role, "name": name, "id": None}
        self.resources.append(resource)  # Track attempted creation even if interrupted.
        identifier = self.docker.text(["network", "create", "--driver", "bridge", "--internal", "--label", f"{LABEL}={self.run_id}", "--label", f"{ROLE_LABEL}={role}", name])
        require(ID.fullmatch(identifier) is not None, "invalid created network ID")
        resource["id"] = identifier
        info = self.docker.inspect("network", identifier)
        require(info is not None, "created network disappeared")
        self.owned(info, role, "network")
        require(info.get("Internal") is True and info.get("Driver") == "bridge", "network must be internal bridge")
        configs = info.get("IPAM", {}).get("Config")
        require(type(configs) is list and len(configs) == 1, "expected one auto-IPAM IPv4 subnet")
        subnet = ipaddress.ip_network(configs[0]["Subnet"])
        require(subnet.version == 4, "kernel NAT fixture is IPv4 only")
        self.networks[role] = name
        self.subnets[role] = str(subnet)

    def create_container(self, role, network_role, image_id):
        name = f"meshway-netcheck-{self.run_id}-{role}"
        resource = {"kind": "container", "role": role, "name": name, "id": None}
        self.resources.append(resource)
        identifier = self.docker.text(container_args(self.run_id, role, name, self.networks[network_role], image_id))
        require(ID.fullmatch(identifier) is not None, "invalid created container ID")
        resource["id"] = identifier
        self.containers[role] = identifier
        if role == "router":
            self.docker.text(["network", "connect", self.networks["wan"], identifier])
        self.docker.text(["container", "start", identifier])
        info = self.docker.inspect("container", identifier)
        require(info is not None, "created container disappeared")
        self.owned(info, role, "container")
        config = info.get("HostConfig", {})
        require(config.get("Privileged") is False and config.get("ReadonlyRootfs") is True, "container isolation missing")
        require(config.get("CapDrop") == ["ALL"] and config.get("CapAdd") in (["NET_ADMIN"], None, []), "unexpected container capabilities")
        require(bool(config.get("CapAdd")) == (role in ("client", "router")), "NET_ADMIN belongs only to client/router")
        require(not info.get("Mounts") and not config.get("PortBindings"), "host mounts or published ports are forbidden")
        networks = info.get("NetworkSettings", {}).get("Networks", {})
        expected = {self.networks[network_role]} | ({self.networks["wan"]} if role == "router" else set())
        require(set(networks) == expected, "unexpected container network attachment")
        for network_name, settings in networks.items():
            address = str(ipaddress.IPv4Address(settings["IPAddress"]))
            network_role = "lan" if network_name == self.networks["lan"] else "wan"
            require(ipaddress.ip_address(address) in ipaddress.ip_network(self.subnets[network_role]), "container IP outside owned subnet")
            self.addresses[f"{role}_{network_role}"] = address

    def execute(self, role, args, timeout=15, checked=True):
        return self.docker.call(["exec", self.containers[role], *args], timeout=timeout, checked=checked)

    def exec_json(self, role, args):
        return strict_json(self.execute(role, args).stdout)

    def provision(self, image_id):
        self.stage = "topology"
        for role in ("lan", "wan"):
            self.create_network(role)
        require(not ipaddress.ip_network(self.subnets["lan"]).overlaps(ipaddress.ip_network(self.subnets["wan"])), "owned LAN and WAN must be distinct")
        for role, network in (("router", "lan"), ("client", "lan"), ("a", "wan"), ("b", "wan")):
            self.create_container(role, network, image_id)
        require(self.addresses["a_wan"] != self.addresses["b_wan"], "observers require distinct WAN IPs")
        self.execute("client", ["ip", "-4", "route", "replace", "default", "via", self.addresses["router_lan"]])
        forwarding = self.execute("router", ["python3", "-B", "-c", "print(open('/proc/sys/net/ipv4/ip_forward').read().strip())"]).stdout.strip()
        require(forwarding == "1", "router namespace forwarding must be enabled")
        route_proofs = []
        for observer in ("a", "b"):
            routes = self.exec_json("client", ["ip", "-j", "-4", "route", "get", self.addresses[f"{observer}_wan"]])
            require(type(routes) is list and len(routes) == 1, "client route proof missing")
            route = routes[0]
            require(route.get("gateway") == self.addresses["router_lan"] and route.get("prefsrc") == self.addresses["client_lan"], "client probe must route through owned router")
            route_proofs.append({"destination": self.addresses[f"{observer}_wan"], "gateway": route["gateway"], "source": route["prefsrc"]})
        self.stage = "observers"
        for role in ("a", "b"):
            for file in ("wire.py", "observer.py"):
                self.docker.text(["cp", str(HERE / file), f"{self.containers[role]}:/private/{file}"])
            self.docker.text(["exec", "--detach", self.containers[role], "python3", "-B", "/private/observer.py", "--bind", self.addresses[f"{role}_wan"], "--subnet", self.subnets["wan"], "--source", self.addresses["router_wan"]])
            ready = False
            for _ in range(50):
                result = self.execute(role, ["test", "-f", "/private/observer.json"], checked=False)
                if result.returncode == 0:
                    self.observation(role)
                    ready = True
                    break
                time.sleep(0.1)
            require(ready, "STUN observer did not become ready")
        return {"subnets": self.subnets, "addresses": self.addresses, "client_routes": route_proofs, "router_ipv4_forwarding": True}

    def observation(self, role):
        value = self.exec_json(role, ["cat", "/private/observer.json"])
        require(type(value) is dict and set(value) == {"ready", "server", "sequence", "events"}, "unexpected observer snapshot")
        require(value["ready"] is True and value["server"] == endpoint(self.addresses[f"{role}_wan"], 3478), "observer server mismatch")
        require(type(value["sequence"]) is int and value["sequence"] >= 0 and type(value["events"]) is list and len(value["events"]) <= 128, "invalid observer sequence")
        previous = 0
        for event in value["events"]:
            require(type(event) is dict and set(event) == {"sequence", "observed_peer"}, "unexpected observer event")
            require(type(event["sequence"]) is int and previous < event["sequence"] <= value["sequence"], "unordered observer event")
            address, _port = parse_endpoint(event["observed_peer"])
            require(str(address) == self.addresses["router_wan"], "observer peer outside source allowlist")
            previous = event["sequence"]
        return value

    def router_counters(self):
        return counters(self.execute("router", ["iptables-save", "-c"]).stdout)

    def filter_canary(self):
        addresses = self.addresses
        # The exited CLI's old conntrack entry must not reserve the fixed SNAT
        # tuple against this new held socket. Only our router namespace is reset.
        self.execute("router", ["conntrack", "-F"])
        for file in ("wire.py", "observer.py", "canary.py"):
            self.docker.text(["cp", str(HERE / file), f"{self.containers['client']}:/private/{file}"])
        nonce = uuid.uuid4().hex
        self.docker.text(["exec", "--detach", self.containers["client"], "python3", "-B", "/private/canary.py", "--client", addresses["client_lan"], "--server-a", addresses["a_wan"], "--server-b", addresses["b_wan"], "--nonce", nonce])
        for _ in range(50):
            ready = self.execute("client", ["test", "-f", "/private/canary.json"], checked=False)
            if ready.returncode == 0:
                break
            time.sleep(0.05)
        require(ready.returncode == 0, "held-socket canary did not become ready")
        state = self.exec_json("client", ["cat", "/private/canary.json"])
        self.validate_canary(state, complete=False)
        before_packets = self.router_counters().get("netcheck-canary", {}).get("packets", 0)
        code = ("import socket; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); "
                f"s.bind(('{addresses['b_wan']}',3479)); "
                f"s.sendto(b'meshway-filter-canary:{nonce}',('{addresses['router_wan']}',41000)); "
                "s.close()")
        self.execute("b", ["python3", "-B", "-c", code])
        self.execute("client", ["python3", "-B", "-c", "from pathlib import Path; Path('/private/canary.sent').touch(mode=0o600)"])
        for _ in range(80):
            state = self.exec_json("client", ["cat", "/private/canary.json"])
            self.validate_canary(state, complete=False)
            if state["complete"]:
                break
            time.sleep(0.05)
        self.validate_canary(state, complete=True)
        proof = self.router_counters()
        require(proof.get("netcheck-canary", {}).get("packets", 0) > before_packets, "unsolicited canary must hit router drop rule")
        require(state["nonce_received"] is False and state["legitimate_responses"] == 2, "filter fixture must reject nonce while accepting A controls")
        observed = self.observation("a")
        require([event["observed_peer"] for event in observed["events"][-2:]] == [state["first_mapping"], state["second_mapping"]], "canary A controls must match actual observer peers")
        self.execute("client", ["python3", "-B", "-c", "from pathlib import Path; [Path('/private',p).unlink() for p in ('wire.py','observer.py','canary.py','canary.json','canary.sent')]"])
        return {"source": endpoint(addresses["b_wan"], 3479), "destination": endpoint(addresses["router_wan"], 41000), "router_drop_counter_increased": True,
                "timing": "same_socket_held_open_between_A_controls", "client_delivery_measured": True,
                "held_socket": state}, proof

    def validate_canary(self, state, complete):
        require(type(state) is dict and set(state) == {"ready", "complete", "bound_socket", "first_mapping", "second_mapping", "legitimate_responses", "nonce_received", "same_socket_open", "receive_window_ms"}, "unexpected canary snapshot")
        require(state["ready"] is True and type(state["complete"]) is bool and type(state["nonce_received"]) is bool and state["same_socket_open"] is True, "invalid held-socket canary state")
        require(type(state["legitimate_responses"]) is int and state["legitimate_responses"] in (1, 2) and type(state["receive_window_ms"]) is int and state["receive_window_ms"] == 700, "invalid canary control count/window")
        address, _port = parse_endpoint(state["bound_socket"])
        require(str(address) == self.addresses["client_lan"] and state["first_mapping"] == endpoint(self.addresses["router_wan"], 41000), "held-socket mapping outside fixture")
        require(state["second_mapping"] in (None, state["first_mapping"]), "held-socket control mapping changed")
        if complete:
            require(state["complete"] is True and state["second_mapping"] == state["first_mapping"], "held-socket canary did not complete")

    def scenario(self, name, ports):
        self.stage = name
        settings = {"scenario": name, "snat_ports_aba": list(ports), "filter_configuration": "default_drop_established_return"}
        if name == "endpoint_restricted_fixture":
            settings["unsolicited_canary_source_port"] = 3479
        addresses = self.addresses
        for command in router_commands(name, addresses["client_lan"], addresses["router_wan"], addresses["a_wan"], addresses["b_wan"], ports):
            self.execute("router", command)
        before = {role: self.observation(role)["sequence"] for role in ("a", "b")}
        servers = [endpoint(addresses[f"{role}_wan"], 3478) for role in ("a", "b")]
        args = ["meshway-agent", "netcheck", "--stun-server", servers[0], "--stun-server", servers[1], "--bind", "0.0.0.0:0", "--timeout-ms", "700", "--json"]
        started = utc_now()
        result = self.execute("client", args, timeout=12, checked=False)
        finished = utc_now()
        report = validate_report(result.stdout, result.returncode, servers, started, finished)
        bound, bound_port = parse_endpoint(report["bound_socket"])
        require(str(bound) in ("0.0.0.0", addresses["client_lan"]) and report["address_family"] == "ipv4", "client bound socket outside fixture")
        # The observer persists immediately after sending; allow snapshot scheduling.
        observed = {}
        for _ in range(20):
            observed = {role: [event["observed_peer"] for event in self.observation(role)["events"] if event["sequence"] > before[role]] for role in ("a", "b")}
            if len(observed["a"]) == sum(port is not None for port in (ports[0], ports[2])) and len(observed["b"]) == int(ports[1] is not None):
                break
            time.sleep(0.05)
        crosscheck(report, addresses["router_wan"], ports, observed)
        tuples = conntrack_mappings(self.execute("router", ["conntrack", "-L", "-p", "udp"]).stdout,
                                   addresses["client_lan"], addresses["router_wan"],
                                   [addresses["a_wan"], addresses["b_wan"]], bound_port, report["samples"])
        proof = self.router_counters()
        for role, port in (("a", ports[0]), ("b", ports[1])):
            if port is not None:
                require(proof.get(f"netcheck-snat-{role}", {}).get("packets", 0) > 0, "kernel SNAT counter must increase")
        if name in ("udp_drop", "partial"):
            label = "netcheck-drop-all" if name == "udp_drop" else "netcheck-drop-b"
            require(proof.get(label, {}).get("packets", 0) > 0, "kernel UDP drop counter must increase")
        if any(port is not None for port in ports):
            require(proof.get("netcheck-return", {}).get("packets", 0) > 0, "stateful return counter must increase")
        canary = None
        if name == "endpoint_restricted_fixture":
            canary, proof = self.filter_canary()
        private_files = self.exec_json("client", ["python3", "-B", "-c", "import json,os; print(json.dumps(os.listdir('/private')))"])
        require(private_files == [], "standalone probe must not create private state")
        return {"settings": settings, "measurement": report, "agent_exit": result.returncode,
                "evidence": {"observed_peers": observed, "router_counters": proof, "conntrack_mappings": tuples, "canary": canary, "private_state_created": False}, "status": "pass"}

    def cleanup(self):
        # Inspect exact attempted names/IDs; never prune or select by a broad label.
        results = []
        for resource in reversed(self.resources):
            status = {"kind": resource["kind"], "role": resource["role"], "status": "failed"}
            try:
                target = resource["id"] or resource["name"]
                info = self.docker.inspect(resource["kind"], target)
                if info is None:
                    status["status"] = "already_absent"
                else:
                    self.owned(info, resource["role"], resource["kind"])
                    identifier = info.get("Id")
                    require(isinstance(identifier, str) and ID.fullmatch(identifier), "invalid owned resource ID")
                    if resource["id"]:
                        require(identifier == resource["id"], "owned resource ID changed")
                    args = ["container", "rm", "--force", identifier] if resource["kind"] == "container" else ["network", "rm", identifier]
                    self.docker.text(args)
                    status["status"] = "removed"
            except Exception:
                # Continue to other exact owned resources; retain structured failure.
                status["status"] = "failed_inspect_or_remove"
            results.append(status)
        return {"complete": all(item["status"] in ("removed", "already_absent") for item in results), "resources": results}


def write_result(directory, result):
    temporary = directory / "result.next"
    temporary.write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
    temporary.replace(directory / "result.json")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True, help="prebuilt dedicated meshway-netcheck:TAG (never pulled)")
    parser.add_argument("--artifact-parent", type=Path, default=TEMP)
    args = parser.parse_args()
    if IMAGE.fullmatch(args.image) is None:
        parser.error("--image must be dedicated meshway-netcheck:TAG")
    os.umask(0o077)
    parent = args.artifact_parent
    require(parent.is_dir() and not parent.is_symlink() and parent.stat().st_uid == os.getuid(), "artifact parent must be an existing owned directory")
    directory = Path(tempfile.mkdtemp(prefix="meshway-netcheck-", dir=parent))
    run_id = uuid.uuid4().hex
    result = {"fixture_schema_version": 1, "run_id": run_id, "status": "failed", "scenarios": [], "cleanup": {"complete": True, "resources": []}}
    signal.signal(signal.SIGINT, interrupt)
    signal.signal(signal.SIGTERM, interrupt)
    exit_code = 1
    try:
        with performance_lock() as descriptor:
            harness = Harness(Docker(descriptor), run_id)
            try:
                image = harness.image(args.image)
                result["image"] = image
                result["topology"] = harness.provision(image["immutable_id"])
                for name, ports in SCENARIOS:
                    result["scenarios"].append(harness.scenario(name, ports))
                older, newer = result["scenarios"][-2:]
                require(older["measurement"]["measured_at"] != newer["measurement"]["measured_at"], "mapping change requires a fresh measurement timestamp")
                result["status"] = "pass"
                exit_code = 0
            except Interrupted:
                result["status"] = "interrupted"
                result["failure_stage"] = harness.stage
                exit_code = 130
            except Exception as error:
                result["failure_stage"] = harness.stage
                result["failure_reason"] = str(error) if isinstance(error, CheckError) else "fixture_command_failed_raw_diagnostics_suppressed"
            finally:
                signal.signal(signal.SIGINT, signal.SIG_IGN)
                signal.signal(signal.SIGTERM, signal.SIG_IGN)
                result["cleanup"] = harness.cleanup()
                if not result["cleanup"]["complete"]:
                    result["status"] = "failed"
                    exit_code = 1
                write_result(directory, result)
    except Interrupted:
        result["status"] = "interrupted"
        result["failure_stage"] = "performance_lock_wait"
        exit_code = 130
        write_result(directory, result)
    except Exception:
        result["failure_reason"] = "preflight_or_artifact_failure_raw_diagnostics_suppressed"
        result["status"] = "failed"
        exit_code = 1
        write_result(directory, result)
    print(json.dumps({"status": result["status"], "artifact": str(directory / "result.json"), "cleanup_complete": result["cleanup"]["complete"]}))
    return exit_code


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        print("netcheck fixture preflight failed; raw diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
