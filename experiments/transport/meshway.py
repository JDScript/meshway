#!/usr/bin/env python3
"""Real Meshway TUN experiment runner; requires a separately built native lab image."""

import argparse
from datetime import datetime, timezone
import ipaddress
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
import stat
import sys
import tempfile
import uuid

from benchmark import (
    Fixture, HERE, LabError, architecture, benchmark_case, bounded_integer,
    bounded_rate, docker, docker_json, interrupted, now, positive_cpus, wait_until, write_result,
)

RUN_DIRECTORY = "/run/meshway-lab"
TUN_NAME = "mwlab0"
TUN_MTU = 1280
MESH_IP = {"NYC": "198.18.0.1", "SHA": "198.18.0.2"}


def numeric_counters(value):
    """Allow counters, not arbitrary future status strings/config/credentials."""
    if not isinstance(value, dict):
        return {}
    result = {}
    for key, item in value.items():
        if not re.fullmatch(r"[a-z][a-z0-9_]*", key):
            continue
        if re.search(r"private|secret|password|credential|uuid|token|key", key):
            continue
        if isinstance(item, (int, float)) and math.isfinite(item):
            result[key] = item
        elif isinstance(item, dict):
            result[key] = numeric_counters(item)
    return result


def public_status(raw):
    if not isinstance(raw, dict) or raw.get("experiment") is not True:
        raise LabError("Agent status must declare experiment=true")
    node_id = raw.get("node_id")
    peers = raw.get("authenticated_peers")
    if not isinstance(node_id, str) or not node_id or len(node_id) > 128:
        raise LabError("Agent status needs a public string node_id")
    if not isinstance(peers, list) or any(not isinstance(peer, str) or len(peer) > 128 for peer in peers):
        raise LabError("Agent status needs authenticated_peers as public node-ID strings")
    result = {"node_id": node_id, "experiment": True, "authenticated_peers": peers}
    if isinstance(raw.get("tun_enabled"), bool):
        result["tun_enabled"] = raw["tun_enabled"]
    for key in ("statistics", "tun"):
        if key in raw:
            result[key] = numeric_counters(raw[key])
    failures = raw.get("vless_handshake_failures")
    if isinstance(failures, int) and failures >= 0:
        result["vless_handshake_failures"] = failures
    return result


def authenticated_graph(statuses):
    if set(statuses) != {"NYC", "LAX", "SHA"}:
        return False
    identities = {node: status["node_id"] for node, status in statuses.items()}
    if len(set(identities.values())) != 3:
        raise LabError("Fixture returned duplicate node identities")
    for node, neighbors in {"NYC": ("LAX",), "LAX": ("NYC", "SHA"), "SHA": ("LAX",)}.items():
        if not all(identities[neighbor] in statuses[node]["authenticated_peers"] for neighbor in neighbors):
            return False
        if "tun_enabled" in statuses[node] and statuses[node]["tun_enabled"] != (node != "LAX"):
            raise LabError(f"Unexpected TUN role in {node} status")
    return True


def cpu_window(before, after):
    """Cgroup v2 usage delta, including all container processes and helper overhead."""
    result = {"before": before, "after": after, "nodes": {}}
    for node in before:
        elapsed = (after[node]["monotonic_ns"] - before[node]["monotonic_ns"]) / 1e9
        entry = {"elapsed_seconds": elapsed}
        samples = []
        for snapshot in (before[node], after[node]):
            fields = snapshot["files"].get("cpu.stat", "")
            samples.append({line.split()[0]: int(line.split()[1]) for line in fields.splitlines() if len(line.split()) == 2})
        if elapsed > 0 and all("usage_usec" in sample for sample in samples):
            delta = samples[1]["usage_usec"] - samples[0]["usage_usec"]
            if delta >= 0:
                entry["usage_usec_delta"] = delta
                entry["cpu_percent_one_core"] = delta / (elapsed * 1e6) * 100
        for key in ("nr_throttled", "throttled_usec"):
            if all(key in sample for sample in samples):
                entry[f"{key}_delta"] = samples[1][key] - samples[0][key]
        result["nodes"][node] = entry
    return result


class MeshwayFixture(Fixture):
    def __init__(self, args, result):
        super().__init__(args, result)
        self.secret_directory = None
        self.agents_started = set()
        self.shutdown_events = []

    def prepare_image(self, native_arch):
        if not self.image:
            raise LabError("Meshway experiments require an already built --image; this runner never builds one")
        super().prepare_image(native_arch)

    def helper(self, node, *args, **kwargs):
        return self.exec(node, "python3", f"{RUN_DIRECTORY}/node.py", *args, **kwargs)

    def agent_helper(self, node, *args, **kwargs):
        return self.exec(node, "python3", f"{RUN_DIRECTORY}/meshway_node.py", *args, **kwargs)

    def create_node(self, node, network, topology, forward=False):
        name = f"{self.prefix}-{topology}-{node.lower()}"
        self.containers_owned.append(name)
        self.nodes[node] = name
        devices = ["--device", "/dev/net/tun"] if node in MESH_IP else []
        docker(
            "run", "--detach", "--name", name, "--hostname", node.lower(), *self.labels(),
            "--label", f"io.meshway.lab.node={node}", "--label", "io.meshway.lab.location=synthetic",
            "--network", self.networks[network]["name"], "--cap-drop", "ALL", "--cap-add", "NET_ADMIN",
            "--cap-add", "NET_RAW", *devices, "--security-opt", "no-new-privileges", "--read-only",
            "--tmpfs", "/run:rw,nosuid,nodev,noexec,mode=0700,size=32m",
            "--tmpfs", "/tmp:rw,nosuid,nodev,noexec,mode=0700,size=16m",
            "--cpus", self.args.cpus, "--memory", self.args.memory, "--memory-swap", self.args.memory,
            "--pids-limit", "64", "--sysctl", "net.ipv4.ip_forward=0",
            "--sysctl", "net.ipv4.conf.all.send_redirects=0", "--sysctl", "net.ipv6.conf.all.disable_ipv6=1",
            "--entrypoint", "python3", self.image, "-c", "import signal; signal.pause()",
        )
        self.exec(node, "mkdir", "-m", "700", RUN_DIRECTORY)
        for filename in ("node.py", "meshway_node.py"):
            self.copy_file(node, HERE / filename, f"{RUN_DIRECTORY}/{filename}")
        docker("exec", "--detach", name, "python3", f"{RUN_DIRECTORY}/node.py", "serve")
        wait_until(f"{node} helper health", lambda: self.probe(node, "127.0.0.1"))

    def generate_configs(self, second_leg):
        self.secret_directory = tempfile.TemporaryDirectory(prefix=f"{self.prefix}-keys-", dir=self.args.temp_dir)
        directory = Path(self.secret_directory.name)
        directory.chmod(0o700)
        generator = f"{self.prefix}-{second_leg}-generator"
        self.containers_owned.append(generator)
        # Only this short-lived, network-less generator sees all three configs.
        generated = docker(
            "run", "--name", generator, *self.labels(), "--network", "none", "--cap-drop", "ALL",
            "--read-only", "--security-opt", "no-new-privileges", "--user", f"{os.getuid()}:{os.getgid()}",
            "--cpus", self.args.cpus, "--memory", self.args.memory, "--memory-swap", self.args.memory,
            "--pids-limit", "64", "--tmpfs", "/tmp:rw,nosuid,nodev,noexec,mode=0700,size=16m",
            "--mount", f"type=bind,source={directory},target=/lab", self.image,
            "transport-lab", "fixture", "--directory", "/lab",
            "--lax-nyc", f"{self.address('LAX', 'a')}:41000",
            "--lax-sha", f"{self.address('LAX', 'b')}:41001", "--second-leg", second_leg, "--tun",
            timeout=30, check=False,
        )
        if generated.returncode:
            # Do not echo generator stdout/stderr: future diagnostics could contain keys.
            raise LabError(f"Fixture generation failed (exit {generated.returncode}); output suppressed to protect credentials")
        docker("container", "rm", generator)
        for node in ("NYC", "LAX", "SHA"):
            config = directory / f"{node.lower()}.json"
            metadata = config.lstat()
            if not stat.S_ISREG(metadata.st_mode) or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_uid != os.getuid():
                raise LabError(f"Generated {node} config must be a private 0600 regular file owned by the invoking user")
            self.copy_file(node, config, f"{RUN_DIRECTORY}/config.json")
        self.secret_directory.cleanup()
        self.secret_directory = None

    def agent_state(self, node):
        return json.loads(self.agent_helper(node, "state").stdout)

    def statuses(self, require_running=True):
        statuses = {}
        for node in self.nodes:
            state = self.agent_state(node)
            if require_running and state["pid_recorded"] and not state["running"]:
                raise LabError(f"{node} agent exited; credentials/logs are not exported")
            response = self.exec(node, "cat", f"{RUN_DIRECTORY}/status.json", check=False)
            if response.returncode:
                continue
            try:
                raw = json.loads(response.stdout)
            except ValueError:
                continue  # A writer without atomic replacement may briefly expose partial JSON.
            statuses[node] = public_status(raw)
        return statuses

    def ping(self, node, peer):
        command = ["ping", "-n", "-I", MESH_IP[node], "-c", "1", "-W", "1", "-w", "2", MESH_IP[peer]]
        result = self.exec(node, *command, check=False, timeout=4)
        return {"command": command, "exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr}

    def tun_ready(self, node):
        response = self.exec(node, "ip", "-json", "address", "show", "dev", TUN_NAME, check=False)
        if response.returncode:
            return False
        interfaces = json.loads(response.stdout)
        return any(interface["mtu"] == TUN_MTU and "UP" in interface["flags"] and
                   any(address.get("local") == MESH_IP[node] and address.get("prefixlen") == 32
                       for address in interface.get("addr_info", [])) for interface in interfaces)

    def connections(self):
        return {node: {
            "tcp_ipv4": json.loads(self.agent_helper(node, "tcp-sockets").stdout),
            "ss_tcp_udp": self.exec(node, "ss", "-H", "-t", "-u", "-n", "-a", "-p").stdout,
        } for node in self.nodes}

    def cgroups(self):
        return {node: json.loads(self.helper(node, "cgroup").stdout) for node in self.nodes}

    def start_agent(self, node):
        self.agents_started.add(node)
        docker("exec", "--detach", self.nodes[node], "python3", f"{RUN_DIRECTORY}/meshway_node.py", "supervise")

    def setup_mesh(self, second_leg):
        self.nodes = {}
        self.networks = {}
        self.shutdown_events = []
        topology = f"mesh-{second_leg}"
        for network in ("a", "b"):
            self.create_network(network, topology)
            subnet = ipaddress.ip_network(self.networks[network]["subnet"])
            if any(ipaddress.ip_address(address) in subnet for address in MESH_IP.values()):
                raise LabError("Docker-allocated underlay overlaps fixed lab TUN addresses; retry with a non-overlapping Docker pool")
        self.create_node("NYC", "a", topology)
        self.create_node("LAX", "a", topology)
        docker("network", "connect", self.networks["b"]["name"], self.nodes["LAX"])
        self.create_node("SHA", "b", topology)
        self.remove_defaults()
        for node, network in (("NYC", "a"), ("SHA", "b")):
            wait_until(f"{node} to LAX underlay health", lambda n=node, net=network: self.probe(n, self.address("LAX", net)))
        if self.probe("NYC", self.address("SHA", "b")) or self.probe("SHA", self.address("NYC", "a")):
            raise LabError("Unexpected endpoint-to-endpoint underlay reachability")
        self.generate_configs(second_leg)
        for node in ("LAX", "NYC", "SHA"):
            self.start_agent(node)
        wait_until("authenticated NYC-LAX-SHA peer graph", lambda: authenticated_graph(self.statuses()), timeout=self.args.ready_timeout)
        for node, peer in (("NYC", "SHA"), ("SHA", "NYC")):
            wait_until(f"{node} addressed/up {TUN_NAME} MTU {TUN_MTU}", lambda n=node: self.tun_ready(n), timeout=self.args.ready_timeout)
            self.exec(node, "ip", "route", "replace", f"{MESH_IP[peer]}/32", "dev", TUN_NAME, "src", MESH_IP[node])
        for node, peer in (("NYC", "SHA"), ("SHA", "NYC")):
            wait_until(f"{node}->{peer} real TUN ICMP", lambda n=node, p=peer: self.ping(n, p)["exit_code"] == 0, timeout=self.args.ready_timeout)
            wait_until(f"{node}->{peer} TUN TCP health", lambda n=node, p=peer: self.probe(n, MESH_IP[p]), timeout=self.args.ready_timeout)
        routes = {}
        for node, peer in (("NYC", "SHA"), ("SHA", "NYC")):
            routes[node] = json.loads(self.exec(node, "ip", "-json", "route", "get", MESH_IP[peer]).stdout)
            if not routes[node] or routes[node][0].get("dev") != TUN_NAME:
                raise LabError(f"{node} mesh route does not use {TUN_NAME}")
        for node in self.nodes:
            if json.loads(self.exec(node, "ip", "-json", "route", "show", "default").stdout):
                raise LabError(f"Unexpected default route on {node}")
        if self.exec("LAX", "cat", "/proc/sys/net/ipv4/ip_forward").stdout.strip() != "0":
            raise LabError("LAX kernel forwarding must remain disabled for the Meshway experiment")
        links = self.connections()
        if second_leg == "vless":
            assert_sha_dials_lax(links["SHA"]["tcp_ipv4"], self.address("SHA", "b"), self.address("LAX", "b"))
        return {
            "authenticated_status": self.statuses(), "mesh_routes": routes,
            "icmp": {"forward": self.ping("NYC", "SHA"), "reverse": self.ping("SHA", "NYC")},
            "endpoint_underlay_blocked": True, "lax_kernel_forwarding": False,
            "sha_dials_lax_vless_verified": second_leg == "vless", "connections": links,
        }

    def stop_agent(self, node):
        self.agent_helper(node, "stop")
        wait_until(f"{node} agent SIGINT shutdown", lambda: not self.agent_state(node)["running"], timeout=8)
        return self.agent_state(node)

    def fault_check(self):
        stopped = self.stop_agent("LAX")
        links = json.loads(self.exec("LAX", "ip", "-json", "link", "show").stdout)
        expected = {self.interface("LAX", self.address("LAX", network)) for network in ("a", "b")}
        if not all(any(link["ifname"] == interface and "UP" in link["flags"] for link in links) for interface in expected):
            raise LabError("LAX underlay interface went down during agent-only fault check")
        for node, network in (("NYC", "a"), ("SHA", "b")):
            if not self.probe(node, self.address("LAX", network)):
                raise LabError("LAX helper became unreachable during agent-only fault check")
        pings = {"forward": self.ping("NYC", "SHA"), "reverse": self.ping("SHA", "NYC")}
        if any(ping["exit_code"] != 1 for ping in pings.values()):
            raise LabError("Stopping only the LAX agent must cause ICMP loss both ways (not a ping command error)")
        return {"passed": True, "lax_agent": stopped, "lax_underlay_links_up": sorted(expected),
                "lax_underlay_health_both_sides": True, "icmp_after_stop": pings}

    def cleanup(self):
        for node in sorted(self.agents_started):
            try:
                self.shutdown_events.append({"node": node, "state": self.stop_agent(node)})
            except (LabError, OSError, ValueError) as error:
                self.shutdown_events.append({"node": node, "graceful_shutdown_error": str(error)})
        self.agents_started.clear()
        errors = super().cleanup()
        if self.secret_directory is not None:
            try:
                self.secret_directory.cleanup()
                self.secret_directory = None
            except OSError as error:
                errors.append(f"Private fixture directory cleanup failed: {error}")
        return errors


def assert_sha_dials_lax(sockets, sha_address, lax_address):
    if not any(connection["state"] == "01" and connection["local"]["address"] == sha_address and
               connection["remote"] == {"address": lax_address, "port": 41001} for connection in sockets):
        raise LabError("Expected established SHA-initiated VLESS TCP connection to LAX:41001")
    if any(connection["state"] == "0A" and connection["local"]["port"] == 41001 for connection in sockets):
        raise LabError("SHA unexpectedly has a static VLESS listener on port 41001")


def baseline_reference(path):
    if path is None:
        return None
    raw = json.loads(path.read_text())
    if raw.get("status") != "ok" or not raw.get("purpose", "").startswith("pure iperf3"):
        raise LabError("Comparison requires a successful pure-iperf3 baseline result")
    routed = next((entry for entry in raw.get("topologies", []) if entry.get("name") == "routed"), None)
    if routed is None:
        raise LabError("Comparison baseline must include the routed NYC-LAX-SHA topology")
    return {"path": str(path), "run_id": raw["run_id"], "image": raw["image"],
            "settings": raw["settings"], "routed": routed}


def compare_case(case, baseline, settings):
    if baseline is None:
        return None
    candidate = next((entry for entry in baseline["routed"]["cases"] if entry["id"] == case["id"] and entry["status"] == "ok"), None)
    if candidate is None:
        return {"case_found": False, "matched_wrapper_baseline": False}
    bare = candidate["summary"]["receiver_bits_per_second"]
    mesh = case["summary"]["receiver_bits_per_second"]
    ratio = mesh / bare if isinstance(bare, (int, float)) and bare > 0 and isinstance(mesh, (int, float)) else None
    keys = ["seconds", "cpus", "memory"]
    if case["protocol"] == "udp":
        keys.extend(["udp_rate", "udp_length"])
    differences = {key: {"baseline": baseline["settings"].get(key), "meshway": settings.get(key)}
                   for key in keys if baseline["settings"].get(key) != settings.get(key)}
    bare_mtu = baseline["settings"].get("mtu", 1500)
    if bare_mtu != TUN_MTU:
        differences["payload_path_mtu"] = {"baseline": bare_mtu, "meshway_inner": TUN_MTU}
    return {"case_found": True, "matched_wrapper_baseline": False,
            "comparison_class": "different_network_semantics",
            "baseline_case_id": candidate["id"], "baseline_receiver_bits_per_second": bare,
            "whole_path_throughput_ratio": ratio, "measurement_setting_differences": differences,
            "semantic_differences": ["kernel IP routing versus userspace encrypted carriers",
                                     "Meshway hop identity, E2E encryption and ordered authorization",
                                     "TUN integration and different buffering/offloads"],
            "interpretation": "Whole-path context only; not incremental wrapper or encryption overhead"}


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", default="meshway-agent-lab:local", help="already built native lab image; never built by this runner")
    parser.add_argument("--output", "--output-file", dest="output", required=True, type=Path)
    parser.add_argument("--second-leg", choices=("both", "udp", "vless"), default="both")
    parser.add_argument("--seconds", type=bounded_integer(1, 30), default=3)
    parser.add_argument("--iterations", type=bounded_integer(1, 10), default=1)
    parser.add_argument("--udp-rate", type=bounded_rate, default="20M")
    parser.add_argument("--udp-length", type=bounded_integer(128, 1252), default=1200, help="inner IPv4 UDP payload (MTU 1280 minus 28 bytes)")
    parser.add_argument("--cpus", type=positive_cpus, default=1.0)
    parser.add_argument("--memory", choices=("128m", "256m", "512m", "1g"), default="256m")
    parser.add_argument("--ready-timeout", type=bounded_integer(5, 120), default=30)
    parser.add_argument("--temp-dir", type=Path, help="approved host scratch directory; defaults to tempfile/TMPDIR")
    parser.add_argument("--baseline-results", type=Path, help="optional routed benchmark.py result for whole-path context, never wrapper overhead")
    parser.add_argument("--run-label", default="functional-smoke", help="operator-supplied evidence label, recorded verbatim")
    return parser.parse_args(argv)


def main(argv=None):
    args = arguments(argv)
    os.umask(0o077)
    output = args.output.expanduser().resolve()
    if output.exists() or not output.parent.is_dir():
        print("Output must be a new file in an existing directory", file=sys.stderr)
        return 2
    signal.signal(signal.SIGTERM, interrupted)
    run_id = f"meshway-lab-{datetime.now(timezone.utc).strftime('%Y%m%d%H%M%S')}-{uuid.uuid4().hex[:8]}"
    result = {
        "schema_version": 2, "run_id": run_id, "started_at": now(), "status": "running",
        "purpose": "Meshway experimental TUN iperf3 comparison, native UDP and VLESS second leg",
        "run_label": args.run_label,
        "packet_settings": {"mesh_inner_mtu": TUN_MTU, "underlay_mtu": 1500, "udp_payload_bytes": args.udp_length},
        "location_semantics": "NYC/LAX/SHA are synthetic nodes inside one local Linux Docker engine",
        "settings": {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()},
        "measurement_notes": [
            "Meshway inner TUN MTU is 1280 over a 1500-byte underlay; bare baselines may use 1500 or matched-payload 1280 links. Check recorded MTUs and per-case differences.",
            "Use the same resolved agent image for baseline and Meshway to match iperf3/package versions. Ratios still include different data paths, not isolated encryption overhead.",
            "Per-case cgroup windows cover all processes in each container, including helper probes and orchestration overhead, not just the agent.",
            "iperf3 CPU fields account for its processes, separately from cgroup CPU; neither measures all host networking work.",
            "This is bounded-rate UDP, not maximum PPS; ICMP through the TUN is not a meshway trace implementation.",
            "Config files, private keys, VLESS credentials, and agent logs are excluded from results.",
        ],
        "experiments": [], "cleanup_errors": [],
    }
    fixture = MeshwayFixture(args, result)
    exit_code = 1
    current = None
    try:
        reference = baseline_reference(args.baseline_results)
        if reference:
            result["baseline_reference"] = {key: reference[key] for key in ("path", "run_id", "image", "settings")}
        try:
            info = docker_json("info", "--format", "{{json .}}", timeout=15)
        except LabError as error:
            raise LabError(f"Linux Docker daemon unavailable. Start OrbStack/Docker and retry.\n{error}") from error
        native_arch = architecture(info.get("Architecture"))
        if info.get("OSType") != "linux" or native_arch != architecture(platform.machine()):
            raise LabError("This experiment requires a local native-architecture Linux Docker daemon")
        result["environment"] = {
            "host": list(platform.uname()), "docker_context": docker("context", "show").stdout.strip(),
            "daemon": {key: info.get(key) for key in (
                "ServerVersion", "OperatingSystem", "OSType", "Architecture", "KernelVersion", "NCPU",
                "MemTotal", "CgroupDriver", "CgroupVersion", "CpuCfsPeriod", "CpuCfsQuota", "Warnings",
            )},
        }
        fixture.prepare_image(native_arch)
        if reference:
            result["baseline_reference"]["same_resolved_image"] = reference["image"]["id"] == fixture.image
        write_result(output, result)
        for second_leg in (("udp", "vless") if args.second_leg == "both" else (args.second_leg,)):
            current = {"second_leg": second_leg, "status": "running", "cases": []}
            result["experiments"].append(current)
            print(f"[{run_id}] Meshway TUN second leg: {second_leg}", flush=True)
            current["readiness"] = fixture.setup_mesh(second_leg)
            current["fixture"] = fixture.snapshot()
            write_result(output, result)
            for iteration in range(1, args.iterations + 1):
                for protocol, parallel in (("tcp", 1), ("tcp", 4), ("udp", 1)):
                    for reverse in (False, True):
                        before = fixture.cgroups()
                        case = benchmark_case(fixture, MESH_IP["NYC"], MESH_IP["SHA"], protocol, parallel, reverse, iteration)
                        current["cases"].append(case)
                        case["container_cpu_window"] = cpu_window(before, fixture.cgroups())
                        case["agent_status_after"] = fixture.statuses()
                        if not authenticated_graph(case["agent_status_after"]):
                            raise LabError("Authenticated peer graph disappeared during the benchmark")
                        if reference:
                            case["baseline_comparison"] = compare_case(case, reference, vars(args))
                        write_result(output, result)
                        rate = case["summary"]["receiver_bits_per_second"]
                        display = f"{rate / 1e6:.2f} Mbit/s" if isinstance(rate, (int, float)) else "unavailable"
                        print(f"  {case['id']}: {display} ({case['status']})", flush=True)
                        if case["status"] != "ok":
                            raise LabError(f"iperf3 failed: {case['id']}; see raw JSON")
            current["connections_after_benchmark"] = fixture.connections()
            current["agent_status_before_fault"] = fixture.statuses()
            current["relay_fault_check"] = fixture.fault_check()
            result["cleanup_errors"].extend(fixture.cleanup())
            current["shutdown"] = list(fixture.shutdown_events)
            if result["cleanup_errors"]:
                raise LabError("Owned-resource cleanup failed; see cleanup_errors")
            current["status"] = "ok"
            write_result(output, result)
            current = None
        result["status"] = "ok"
        exit_code = 0
    except (LabError, OSError, ValueError, KeyError) as error:
        result["status"] = "failed"
        result["error"] = str(error)
        print(str(error), file=sys.stderr)
    except KeyboardInterrupt:
        result["status"] = "interrupted"
        result["error"] = "Interrupted; attempting owned-resource and private-config cleanup"
        exit_code = 130
    finally:
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        if current is not None:
            current["status"] = result["status"]
        result["cleanup_errors"].extend(fixture.cleanup())
        if current is not None:
            current["shutdown"] = list(fixture.shutdown_events)
        if result["cleanup_errors"]:
            result["status"] = "cleanup_failed"
            exit_code = 1
        result["finished_at"] = now()
        write_result(output, result)
        print(f"Results: {output} ({result['status']})", flush=True)
    return exit_code


if __name__ == "__main__":
    sys.path.insert(0, str(HERE.parents[1] / "scripts"))
    from performance_lock import performance_lock
    with performance_lock():
        sys.exit(main())
