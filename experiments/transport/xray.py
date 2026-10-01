#!/usr/bin/env python3
"""Pure Xray L4 TCP relay experiment; raw VLESS, verified TLS, and plain relay control."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import signal
import sys
import tempfile
import uuid

from benchmark import (
    Fixture, HERE, LabError, architecture, bounded_integer, docker, docker_json,
    interrupted, now, positive_cpus, summarize, wait_until, write_result,
)
from meshway import cpu_window
from xray_node import HEALTH_PORT, ROOT, TLS_NAME

MODES = ("vless-tcp", "vless-tls", "tcp-relay")
XRAY_SOURCE = "ghcr.io/xtls/xray-core@sha256:592ec4d11f656db95598d01e76dbcc6e002d67360b96a5436500a938230f52c7"


def tunnel(tag, listen, port, target, target_port):
    return {"tag": tag, "listen": listen, "port": port, "protocol": "dokodemo-door",
            "settings": {"address": target, "port": target_port, "network": "tcp", "followRedirect": False}}


def configurations(mode, credential, lax, sha):
    if mode not in MODES:
        raise ValueError("Unsupported Xray mode")
    # Blackhole first is defense in depth: unmatched requests cannot use freedom.
    server = {"log": {"access": "none", "loglevel": "error"}, "outbounds": [
        {"tag": "deny", "protocol": "blackhole", "settings": {}},
        {"tag": "allowed", "protocol": "freedom", "settings": {}},
    ], "routing": {"domainStrategy": "AsIs", "rules": [
        {"type": "field", "ip": [f"{sha}/32"], "port": f"5201,{HEALTH_PORT}",
         "network": "tcp", "outboundTag": "allowed"},
        {"type": "field", "network": "tcp,udp", "outboundTag": "deny"},
    ]}}
    client = {"log": {"access": "none", "loglevel": "error"}, "inbounds": [
        tunnel("iperf", "127.0.0.1", 15201, sha, 5201),
        tunnel("health", "127.0.0.1", HEALTH_PORT, sha, HEALTH_PORT),
    ]}
    if mode == "tcp-relay":
        server["inbounds"] = [tunnel("iperf", lax, 41001, sha, 5201),
                              tunnel("health", lax, 41002, sha, HEALTH_PORT)]
        client["inbounds"][0]["settings"].update(address=lax, port=41001)
        client["inbounds"][1]["settings"].update(address=lax, port=41002)
        client["outbounds"] = [{"tag": "relay", "protocol": "freedom", "settings": {}}]
    else:
        server["inbounds"] = [{"tag": "vless", "listen": lax, "port": 41001, "protocol": "vless",
                               "settings": {"clients": [{"id": credential}], "decryption": "none"},
                               "streamSettings": {"network": "tcp", "security": "none"}}]
        client["outbounds"] = [{"tag": "relay", "protocol": "vless", "mux": {"enabled": False},
                                "settings": {"vnext": [{"address": lax, "port": 41001,
                                    "users": [{"id": credential, "encryption": "none"}]}]},
                                "streamSettings": {"network": "tcp", "security": "none"}}]
        if mode == "vless-tls":
            server["inbounds"][0]["streamSettings"].update(security="tls", tlsSettings={
                "minVersion": "1.3", "maxVersion": "1.3", "certificates": [{
                    "certificateFile": f"{ROOT}/server.crt", "keyFile": f"{ROOT}/server.key"}],
            })
            client["outbounds"][0]["streamSettings"].update(security="tls", tlsSettings={
                "serverName": TLS_NAME, "allowInsecure": False, "disableSystemRoot": True,
                "minVersion": "1.3", "maxVersion": "1.3",
                "certificates": [{"certificateFile": f"{ROOT}/ca.crt", "usage": "verify"}],
            })
    return {"NYC": client, "LAX": server}


def connected(sockets, source, target, port):
    return any(entry["state"] == "01" and entry["local"]["address"] == source
               and entry["remote"] == {"address": target, "port": port} for entry in sockets)


def iperf_path_check(client, server, parallel, lax, sha):
    client_start, server_start = client.get("start", {}), server.get("start", {})
    clients, servers = client_start.get("connected", []), server_start.get("connected", [])
    control = client_start.get("connecting_to") == {"host": "127.0.0.1", "port": 15201}
    control = control and server_start.get("accepted_connection", {}).get("host") == lax
    streams = (len(clients) == parallel and len(servers) == parallel
               and all(entry.get("local_host") == "127.0.0.1" and entry.get("remote_host") == "127.0.0.1"
                       and entry.get("remote_port") == 15201 for entry in clients)
               and all(entry.get("remote_host") == lax and entry.get("local_host") == sha
                       and entry.get("local_port") == 5201 for entry in servers))
    return {"passed": bool(control and streams), "control_peer_is_lax": bool(control),
            "all_data_streams_use_proxy": bool(streams), "expected_data_streams": parallel,
            "client_data_streams": len(clients), "server_data_streams": len(servers)}


def socket_counter_delta(before, after):
    def parse(snapshot):
        values = {}
        for filename in ("snmp", "netstat"):
            lines = snapshot["files"][filename].splitlines()
            for header, counters in zip(lines[::2], lines[1::2]):
                names, numbers = header.split(), counters.split()
                if names[0] != numbers[0] or len(names) != len(numbers):
                    raise LabError("Malformed /proc/net TCP/IP counters")
                for name, number in zip(names[1:], numbers[1:]):
                    values[f"{names[0].rstrip(':')}.{name}"] = int(number)
        return values

    deltas = {}
    for node in before:
        start, end = parse(before[node]), parse(after[node])
        deltas[node] = {key: value - start[key] for key, value in end.items()
                        if key in start and (key.startswith("Tcp.") or key.startswith("TcpExt."))}
    return {"before": before, "after": after, "tcp_deltas": deltas}


class XrayFixture(Fixture):
    def __init__(self, args, result):
        super().__init__(args, result)
        self.secret_directory = None
        self.started = set()

    def prepare_image(self, native_arch):
        if not self.image:
            raise LabError("Build Dockerfile.xray separately and supply --image")
        super().prepare_image(native_arch)
        labels = docker_json("image", "inspect", self.image)[0]["Config"].get("Labels", {})
        if labels.get("io.meshway.xray.source") != XRAY_SOURCE:
            raise LabError("Image must contain the pinned Xray build from Dockerfile.xray")
        self.result["image"]["xray_source"] = labels["io.meshway.xray.source"]
        self.result["image"]["lab_base_reference"] = labels.get("io.meshway.xray.lab-base")
        base_id = labels.get("io.meshway.xray.lab-base-id")
        if not base_id:
            raise LabError("Image must record its exact lab base ID")
        base = docker_json("image", "inspect", base_id)[0]
        image = docker_json("image", "inspect", self.image)[0]
        base_layers = base["RootFS"]["Layers"]
        if image["RootFS"]["Layers"][:len(base_layers)] != base_layers:
            raise LabError("Xray image does not preserve the declared lab base filesystem layers")
        self.result["image"]["lab_base_id"] = base["Id"]
        self.result["image"]["base_layers_verified"] = True

    def xhelper(self, node, *args, **kwargs):
        return self.exec(node, "python3", f"{ROOT}/xray_node.py", *args, **kwargs)

    def create_node(self, node, network, topology, forward=False):
        super().create_node(node, network, topology, forward=False)
        self.exec(node, "mkdir", "-m", "700", str(ROOT))
        self.copy_file(node, HERE / "xray_node.py", f"{ROOT}/xray_node.py")

    def generate_configs(self, mode):
        self.secret_directory = tempfile.TemporaryDirectory(prefix=f"{self.prefix}-xray-", dir=self.args.temp_dir)
        directory = Path(self.secret_directory.name)
        directory.chmod(0o700)
        if mode == "vless-tls":
            self.xhelper("LAX", "generate-tls", self.address("LAX", "a"), timeout=45)
            ca = directory / "ca.crt"
            ca.write_text(self.exec("LAX", "cat", f"{ROOT}/ca.crt").stdout)
            ca.chmod(0o600)
            self.copy_file("NYC", ca, f"{ROOT}/ca.crt")
        configs = configurations(mode, str(uuid.uuid4()), self.address("LAX", "a"), self.address("SHA", "b"))
        for node, config in configs.items():
            path = directory / f"{node.lower()}.json"
            path.write_text(json.dumps(config))
            path.chmod(0o600)
            self.copy_file(node, path, f"{ROOT}/config.json")
            if self.xhelper(node, "validate", check=False).returncode:
                raise LabError(f"{node} Xray rejected the private config; diagnostics suppressed")
        self.secret_directory.cleanup()
        self.secret_directory = None

    def states(self):
        return {node: json.loads(self.xhelper(node, "state").stdout) for node in ("NYC", "LAX")}

    def cgroups(self):
        return {node: json.loads(self.helper(node, "cgroup").stdout) for node in self.nodes}

    def socket_stats(self):
        return {node: json.loads(self.xhelper(node, "socket-stats").stdout) for node in self.nodes}

    def proxy_probe(self):
        return self.xhelper("NYC", "health-probe", check=False, timeout=4).returncode == 0

    def setup_xray(self, mode):
        self.nodes = {}
        self.networks = {}
        self.create_network("a", mode)
        self.create_network("b", mode)
        self.create_node("NYC", "a", mode)
        self.create_node("LAX", "a", mode)
        docker("network", "connect", self.networks["b"]["name"], self.nodes["LAX"])
        self.create_node("SHA", "b", mode)
        self.remove_defaults()
        for node in self.nodes:
            links = json.loads(self.exec(node, "ip", "-json", "link", "show").stdout)
            if any(link["mtu"] != self.args.mtu for link in links if link["ifname"] != "lo"):
                raise LabError(f"Requested underlay MTU was not applied to {node}")
            if self.exec(node, "cat", "/proc/sys/net/ipv4/ip_forward").stdout.strip() != "0":
                raise LabError(f"Kernel forwarding must be disabled on {node}")
            if json.loads(self.exec(node, "ip", "-json", "route", "show", "default").stdout):
                raise LabError(f"Unexpected default route on {node}")
        source, target = self.address("NYC", "a"), self.address("SHA", "b")
        blocked_routes = {}
        for node, address in (("NYC", target), ("SHA", source)):
            route = self.exec(node, "ip", "route", "get", address, check=False)
            if route.returncode == 0 or self.probe(node, address):
                raise LabError("Unexpected endpoint-to-endpoint underlay route/reachability")
            blocked_routes[node] = {"exit_code": route.returncode, "stderr": route.stderr.strip()}
        for node, network in (("NYC", "a"), ("SHA", "b")):
            if not self.probe(node, self.address("LAX", network)):
                raise LabError(f"{node} cannot reach the LAX underlay helper")
        docker("exec", "--detach", self.nodes["SHA"], "python3", f"{ROOT}/xray_node.py", "health-serve", target)
        wait_until("SHA health listener", lambda: self.helper("SHA", "listening", str(HEALTH_PORT), check=False).returncode == 0)
        self.generate_configs(mode)
        for node in ("LAX", "NYC"):
            self.started.add(node)
            docker("exec", "--detach", self.nodes[node], "python3", f"{ROOT}/xray_node.py", "supervise")
        wait_until("Xray end-to-end health payload", self.proxy_probe, timeout=self.args.ready_timeout)
        if not all(state["running"] for state in self.states().values()):
            raise LabError("Xray process exited during readiness")
        proof = {"endpoint_underlay_routes_blocked": blocked_routes, "all_kernel_forwarding_disabled": True,
                 "proxy_health_payload_received": True, "xray_processes": self.states(),
                 "xray_version": self.exec("NYC", "xray", "version").stdout.strip()}
        if mode == "vless-tls":
            proof["tls"] = json.loads(self.xhelper("NYC", "tls-check", self.address("LAX", "a")).stdout)
            proof["tls"]["xray_verified_proxy_payload_received"] = True
        # Keep one real application stream open while observing both transport legs.
        docker("exec", "--detach", self.nodes["NYC"], "python3", f"{ROOT}/xray_node.py", "health-hold")

        def holding():
            response = self.exec("NYC", "cat", f"{ROOT}/held-health.json", check=False)
            return response.returncode == 0 and json.loads(response.stdout)["connected"]

        try:
            wait_until("held health stream", holding)
            proof["held_stream_sockets"] = self.socket_stats()
            nyc = proof["held_stream_sockets"]["NYC"]["tcp_ipv4"]
            lax = proof["held_stream_sockets"]["LAX"]["tcp_ipv4"]
            relay_port = 41002 if mode == "tcp-relay" else 41001
            if not connected(nyc, source, self.address("LAX", "a"), relay_port):
                raise LabError("Missing established NYC->LAX proxy socket")
            if not connected(lax, self.address("LAX", "b"), target, HEALTH_PORT):
                raise LabError("Missing established LAX->SHA application socket")
            proof["both_proxy_legs_observed"] = True
        finally:
            self.exec("NYC", "touch", f"{ROOT}/release-health")
        wait_until("held health stream release", lambda: not holding())
        return proof

    def stop_xray(self, node):
        self.xhelper(node, "stop")
        wait_until(f"{node} Xray shutdown", lambda: not json.loads(self.xhelper(node, "state").stdout)["running"], timeout=8)
        return json.loads(self.xhelper(node, "state").stdout)

    def fault_check(self):
        stopped = self.stop_xray("LAX")
        self.started.discard("LAX")
        links = json.loads(self.exec("LAX", "ip", "-json", "link", "show").stdout)
        expected = {self.interface("LAX", self.address("LAX", network)) for network in ("a", "b")}
        if not all(any(link["ifname"] == interface and "UP" in link["flags"] for link in links) for interface in expected):
            raise LabError("LAX underlay links changed during Xray-only fault check")
        for node, network in (("NYC", "a"), ("SHA", "b")):
            if not self.probe(node, self.address("LAX", network)):
                raise LabError("LAX helper stopped during Xray-only fault check")
        if not self.states()["NYC"]["running"] or self.proxy_probe():
            raise LabError("Expected NYC Xray alive and fresh proxy payload blocked")
        target = self.address("SHA", "b")
        docker("exec", "--detach", self.nodes["SHA"], "iperf3", "--server", "--one-off", "--bind", target,
               "--port", "5201", "--json", "--logfile", "/run/fault-server.json")
        wait_until("fault iperf listener", lambda: self.helper("SHA", "listening", "5201", check=False).returncode == 0)
        control = json.loads(self.xhelper("NYC", "fault-client", timeout=8).stdout)
        if not control["blocked"]:
            raise LabError(f"Fresh iperf3 fault check did not report a recognized connection failure: {json.dumps(control)}")
        return {"passed": True, "lax_xray": stopped, "lax_underlay_links_up": sorted(expected),
                "lax_underlay_helper_alive_both_sides": True, "nyc_xray_running": True,
                "new_proxy_payload_blocked": True, "new_iperf_control": control}

    def cleanup(self):
        # Container removal remains the authoritative fallback if a child cannot stop.
        for node in sorted(self.started):
            try:
                self.stop_xray(node)
            except (LabError, ValueError, OSError):
                pass
        self.started.clear()
        errors = super().cleanup()
        if self.secret_directory is not None:
            try:
                self.secret_directory.cleanup()
                self.secret_directory = None
            except OSError as error:
                errors.append(f"Private Xray directory cleanup failed: {error}")
        return errors


def case_commands(args, target, parallel, reverse, case_id):
    server_file = f"/run/{case_id}.json"
    server = ["iperf3", "--server", "--one-off", "--bind", target, "--port", "5201", "--json", "--logfile", server_file]
    client = ["iperf3", "--client", "127.0.0.1", "--bind", "127.0.0.1", "--port", "15201",
              "--time", str(args.seconds), "--omit", str(args.omit), "--parallel", str(parallel),
              "--json", "--get-server-output", "--connect-timeout", "3000"]
    if reverse:
        client.append("--reverse")
    return server, client, server_file


def benchmark_xray_case(fixture, parallel, reverse, iteration):
    args = fixture.args
    case_id = f"tcp-p{parallel}-{'reverse' if reverse else 'forward'}-{iteration}"
    server, client, server_file = case_commands(args, fixture.address("SHA", "b"), parallel, reverse, case_id)
    docker("exec", "--detach", fixture.nodes["SHA"], *server)
    wait_until("iperf3 listener", lambda: fixture.helper("SHA", "listening", "5201", check=False).returncode == 0)
    before_sockets = fixture.socket_stats()
    before_cpu = fixture.cgroups()
    started = now()
    execution = fixture.exec("NYC", *client, timeout=args.seconds + args.omit + 20, check=False)
    after_cpu = fixture.cgroups()
    after_sockets = fixture.socket_stats()
    try:
        raw = json.loads(execution.stdout)
    except ValueError as error:
        raise LabError("Invalid iperf3 client JSON") from error
    server_raw = []

    def server_complete():
        content = fixture.exec("SHA", "cat", server_file, check=False)
        if content.returncode:
            return False
        try:
            parsed = json.loads(content.stdout)
        except ValueError:
            return False
        server_raw.append(parsed)
        return True

    wait_until("iperf3 server JSON", server_complete, timeout=8)
    states = fixture.states()
    path_check = iperf_path_check(raw, server_raw[0], parallel, fixture.address("LAX", "b"), fixture.address("SHA", "b"))
    status = "ok" if (execution.returncode == 0 and not raw.get("error") and not server_raw[0].get("error")
                      and all(state["running"] for state in states.values()) and path_check["passed"]) else "failed"
    return {"id": case_id, "protocol": "tcp", "parallel": parallel, "reverse": reverse, "iteration": iteration,
            "started_at": started, "finished_at": now(), "status": status,
            "traffic_direction": "SHA->NYC" if reverse else "NYC->SHA", "control_direction": "NYC->SHA",
            "client_command": client, "server_command": server, "exit_code": execution.returncode,
            "stderr": execution.stderr, "raw_client": raw, "raw_server": server_raw[0], "summary": summarize(raw),
            "server_iperf_process_cpu_percent": server_raw[0].get("end", {}).get("cpu_utilization_percent"),
            "container_cpu_window": cpu_window(before_cpu, after_cpu),
            "socket_window": socket_counter_delta(before_sockets, after_sockets), "xray_processes_after": states,
            "proxy_path_check": path_check}


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", default="meshway-xray-lab:local", help="prebuilt Dockerfile.xray image; resolved ID is frozen")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--mode", choices=("all", *MODES), default="all")
    parser.add_argument("--seconds", type=bounded_integer(1, 30), default=5)
    parser.add_argument("--omit", type=bounded_integer(0, 5), default=1, help="unmeasured iperf warmup seconds per case")
    parser.add_argument("--iterations", type=bounded_integer(1, 10), default=3)
    parser.add_argument("--parallel", choices=("both", "1", "4"), default="both")
    parser.add_argument("--direction", choices=("both", "forward", "reverse"), default="both")
    parser.add_argument("--cpus", type=positive_cpus, default=1.0)
    parser.add_argument("--memory", choices=("128m", "256m", "512m", "1g"), default="256m")
    parser.add_argument("--mtu", choices=(1280, 1500), type=int, default=1500)
    parser.add_argument("--ready-timeout", type=bounded_integer(5, 60), default=15)
    parser.add_argument("--temp-dir", type=Path, help="private config parent; defaults to tempfile/TMPDIR")
    parser.add_argument("--run-label", default="local-xray-comparison")
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
    result = {"schema_version": 1, "run_id": run_id, "started_at": now(), "status": "running",
              "purpose": "pure Xray L4 TCP relay comparison; no Meshway data plane or TUN",
              "run_label": args.run_label,
              "location_semantics": "NYC/LAX/SHA are synthetic nodes inside one local Linux Docker engine",
              "settings": {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()},
              "measurement_notes": [
                  "VLESS/TCP has UUID authentication but no payload encryption; tcp-relay has neither.",
                  "VLESS/TLS uses a private test CA, verified SAN, and TLS 1.3 on NYC->LAX only; LAX->SHA is plaintext.",
                  "L4 streams terminate TCP at proxies. Kernel GSO/GRO remain available; there is no TUN or Noise envelope.",
                  "These modes do not provide Meshway end-to-end IP security. Ratios cannot isolate encryption cost.",
                  "One quota per container: NYC includes Xray plus iperf3, LAX Xray, SHA iperf3; helper cost is included.",
                  "Cgroup windows include omit/warmup and orchestration. iperf summaries exclude omit seconds.",
                  "CPU windows do not account for all host/bridge networking work; quotas are not pinned dedicated cores.",
                  "TCP/IP counter deltas cover each network namespace; CurrEstab and some fields are gauges, not event counts.",
                  "Mux, XUDP, UDP, Vision, REALITY, TUN, and Meshway are not enabled in this harness.",
                  "No management network or published ports. All experiment credentials/log payloads are excluded from JSON.",
              ], "experiments": [], "cleanup_errors": []}
    fixture = XrayFixture(args, result)
    current = None
    exit_code = 1
    try:
        info = docker_json("info", "--format", "{{json .}}", timeout=15)
        native_arch = architecture(info.get("Architecture"))
        if info.get("OSType") != "linux" or native_arch != architecture(platform.machine()):
            raise LabError("This experiment requires a local native-architecture Linux Docker daemon")
        result["environment"] = {
            "host": list(platform.uname()), "docker_context": docker("context", "show").stdout.strip(),
            "daemon": {key: info.get(key) for key in ("ServerVersion", "OperatingSystem", "OSType", "Architecture",
                       "KernelVersion", "NCPU", "MemTotal", "CgroupDriver", "CgroupVersion", "Warnings")},
        }
        fixture.prepare_image(native_arch)
        write_result(output, result)
        for mode in MODES if args.mode == "all" else (args.mode,):
            print(f"[{run_id}] Pure Xray: {mode}", flush=True)
            current = {"mode": mode, "status": "running", "cases": []}
            result["experiments"].append(current)
            current["readiness"] = fixture.setup_xray(mode)
            current["fixture"] = fixture.snapshot()
            write_result(output, result)
            for iteration in range(1, args.iterations + 1):
                for parallel in (1, 4) if args.parallel == "both" else (int(args.parallel),):
                    for reverse in (False, True) if args.direction == "both" else (args.direction == "reverse",):
                        case = benchmark_xray_case(fixture, parallel, reverse, iteration)
                        current["cases"].append(case)
                        write_result(output, result)
                        print(f"  {case['id']}: {case['status']}", flush=True)
                        if case["status"] != "ok":
                            raise LabError(f"iperf3 failed: {case['id']}; see raw JSON")
            current["relay_fault_check"] = fixture.fault_check()
            result["cleanup_errors"].extend(fixture.cleanup())
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
        if result["cleanup_errors"]:
            result["status"] = "cleanup_failed"
            exit_code = 1
        result["finished_at"] = now()
        write_result(output, result)
        print(f"Results: {output} ({result['status']})", flush=True)
    return exit_code


if __name__ == "__main__":
    from performance_lock import performance_lock
    with performance_lock():
        sys.exit(main())
