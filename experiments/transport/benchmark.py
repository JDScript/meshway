#!/usr/bin/env python3
"""Bounded, native-Linux iperf3 baselines in disposable Docker networks."""

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
import subprocess
import sys
import tempfile
import time
import uuid

HERE = Path(__file__).resolve().parent
BASE_IMAGE = "alpine:3.22.1@sha256:4bcff63911fcb4448bd4fdacec207030997caf25e9bea4045fa6c8c44de311d1"
LABEL = "io.meshway.experiment"
RUN_LABEL = "io.meshway.lab.run"
NODE_HELPER = "/run/transport-lab/node.py"


class LabError(RuntimeError):
    pass


def now():
    return datetime.now(timezone.utc).isoformat()


def architecture(value):
    return {"aarch64": "arm64", "arm64": "arm64", "x86_64": "amd64", "amd64": "amd64"}.get(value, value)


def docker(*args, timeout=20, check=True):
    command = ["docker", *map(str, args)]
    try:
        result = subprocess.run(command, text=True, capture_output=True, timeout=timeout)
    except FileNotFoundError as error:
        raise LabError("Docker CLI is unavailable. Install/start a local Linux Docker engine.") from error
    except subprocess.TimeoutExpired as error:
        raise LabError(f"Docker command timed out after {timeout}s: {' '.join(command)}") from error
    if check and result.returncode:
        raise LabError(f"Command failed ({result.returncode}): {' '.join(command)}\n{result.stderr.strip()}\n{result.stdout.strip()}")
    return result


def docker_json(*args, **kwargs):
    return json.loads(docker(*args, **kwargs).stdout)


def write_result(path, result):
    """Atomically save private output next to its destination, never in the repo by default."""
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, prefix=f".{path.name}.", delete=False) as handle:
        temporary = Path(handle.name)
        try:
            json.dump(result, handle, indent=2, allow_nan=False)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def wait_until(description, predicate, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        if predicate():
            return
        if time.monotonic() >= deadline:
            raise LabError(f"Timed out waiting for {description}")
        time.sleep(0.1)  # Bounded readiness polling, not a startup delay.


class Fixture:
    """Owns only run-labelled resources. Docker exec is the out-of-band control path.

    Transport adapters can reuse exec(), address(), interface(), snapshot(), and the
    two-network topology; never add a shared management bridge between endpoints.
    """

    def __init__(self, args, result):
        self.args = args
        self.result = result
        self.prefix = result["run_id"]
        self.image = args.image
        self.owned_image = None
        self.nodes = {}
        self.networks = {}
        self.containers_owned = []
        self.networks_owned = []

    def labels(self):
        return ["--label", f"{LABEL}=transport", "--label", f"{RUN_LABEL}={self.prefix}"]

    def copy_file(self, node, source, destination):
        # The archive/cp API does not reliably traverse this engine's tmpfs.
        # Streaming a single file also keeps future private configs out of argv.
        writer = (
            "import os, shutil, sys\n"
            "os.umask(0o077)\n"
            "fd = os.open(sys.argv[1], os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)\n"
            "with os.fdopen(fd, 'wb') as output:\n"
            "    shutil.copyfileobj(sys.stdin.buffer, output)\n"
        )
        try:
            with source.open("rb") as incoming:
                copied = subprocess.run(
                    ["docker", "exec", "-i", self.nodes[node], "python3", "-c", writer, destination],
                    stdin=incoming, capture_output=True, timeout=20,
                )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise LabError(f"Private file transfer to {node} failed ({type(error).__name__}); content not exported") from error
        if copied.returncode:
            raise LabError(f"Private file transfer to {node} failed (exit {copied.returncode}); content not exported")

    def prepare_image(self, native_arch):
        if not self.image:
            self.image = f"{self.prefix}:lab"
            self.owned_image = self.image
            built = docker(
                "build", "--platform", f"linux/{native_arch}", "--build-arg", f"BASE_IMAGE={self.args.base_image}",
                "--build-arg", f"LAB_RUN_ID={self.prefix}", "--tag", self.image, HERE,
                timeout=self.args.build_timeout,
            )
            self.result["build_log"] = built.stdout + built.stderr
        image = docker_json("image", "inspect", self.image)[0]
        if image["Os"] != "linux" or architecture(image["Architecture"]) != native_arch:
            raise LabError(f"Refusing emulated/non-Linux image: {image['Os']}/{image['Architecture']}; daemon is linux/{native_arch}")
        self.result["image"] = {
            "reference": self.image, "id": image["Id"], "repo_digests": image.get("RepoDigests", []),
            "architecture": image["Architecture"], "os": image["Os"], "created": image["Created"],
            "base_reference": self.args.base_image if not self.args.image else None,
        }
        # Freeze the resolved ID for every container, even if a supplied tag moves.
        self.image = image["Id"]

    def create_network(self, key, topology):
        name = f"{self.prefix}-{topology}-{key}"
        self.networks_owned.append(name)  # Track before creation to cover ambiguous CLI failures.
        mtu = getattr(self.args, "mtu", 1500)
        docker("network", "create", "--driver", "bridge", "--internal", "--opt", f"com.docker.network.driver.mtu={mtu}", *self.labels(), name)
        info = docker_json("network", "inspect", name)[0]
        ipv4 = [entry for entry in info["IPAM"]["Config"] if ipaddress.ip_network(entry["Subnet"]).version == 4]
        if len(ipv4) != 1:
            raise LabError(f"Expected one Docker-allocated IPv4 subnet for {name}")
        self.networks[key] = {"name": name, "id": info["Id"], "subnet": ipv4[0]["Subnet"], "internal": info["Internal"], "requested_mtu": mtu}

    def create_node(self, node, network, topology, forward=False):
        name = f"{self.prefix}-{topology}-{node.lower()}"
        self.containers_owned.append(name)
        self.nodes[node] = name
        docker(
            "run", "--detach", "--name", name, "--hostname", node.lower(), *self.labels(),
            "--label", f"io.meshway.lab.node={node}", "--label", "io.meshway.lab.location=synthetic",
            "--network", self.networks[network]["name"], "--cap-drop", "ALL", "--cap-add", "NET_ADMIN",
            "--security-opt", "no-new-privileges", "--read-only", "--tmpfs", "/run:rw,nosuid,nodev,noexec,mode=0700,size=16m",
            "--tmpfs", "/tmp:rw,nosuid,nodev,noexec,mode=0700,size=16m", "--cpus", self.args.cpus,
            "--memory", self.args.memory, "--memory-swap", self.args.memory, "--pids-limit", "64",
            "--sysctl", f"net.ipv4.ip_forward={int(forward)}",
            "--sysctl", "net.ipv4.conf.all.send_redirects=0",
            "--sysctl", "net.ipv4.conf.default.send_redirects=0",
            "--sysctl", "net.ipv6.conf.all.disable_ipv6=1", "--entrypoint", "python3", self.image,
            "-c", "import signal; signal.pause()",
        )
        self.exec(node, "mkdir", "-m", "700", str(Path(NODE_HELPER).parent))
        self.copy_file(node, HERE / "node.py", NODE_HELPER)
        docker("exec", "--detach", name, "python3", NODE_HELPER, "serve")
        wait_until(f"{node} health service", lambda: self.probe(node, "127.0.0.1"))

    def exec(self, node, *args, **kwargs):
        return docker("exec", self.nodes[node], *args, **kwargs)

    def helper(self, node, *args, **kwargs):
        return self.exec(node, "python3", NODE_HELPER, *args, **kwargs)

    def probe(self, node, address):
        return self.helper(node, "probe", address, check=False, timeout=4).returncode == 0

    def address(self, node, network):
        info = docker_json("container", "inspect", self.nodes[node])[0]
        return info["NetworkSettings"]["Networks"][self.networks[network]["name"]]["IPAddress"]

    def interface(self, node, address):
        interfaces = json.loads(self.exec(node, "ip", "-json", "address", "show").stdout)
        for interface in interfaces:
            if any(entry.get("local") == address for entry in interface["addr_info"]):
                return interface["ifname"]
        raise LabError(f"No interface with address {address} on {node}")

    def remove_defaults(self):
        for node in self.nodes:
            defaults = json.loads(self.exec(node, "ip", "-json", "route", "show", "default").stdout)
            for route in defaults:
                self.exec(node, "ip", "route", "del", "default", "dev", route["dev"])

    def snapshot(self):
        nodes = {}
        for node, name in self.nodes.items():
            info = docker_json("container", "inspect", name)[0]
            config = info["HostConfig"]
            nodes[node] = {
                "container": name,
                "networks": info["NetworkSettings"]["Networks"],
                "limits": {key: config.get(key) for key in ("NanoCpus", "CpuQuota", "CpuPeriod", "CpusetCpus", "Memory", "MemorySwap", "PidsLimit")},
                "capabilities_added": config["CapAdd"], "capabilities_dropped": config["CapDrop"],
                "privileged": config["Privileged"], "sysctls": config["Sysctls"],
                "addresses": json.loads(self.exec(node, "ip", "-json", "address", "show").stdout),
                "routes": json.loads(self.exec(node, "ip", "-json", "route", "show", "table", "all").stdout),
                "runtime": json.loads(self.helper(node, "info").stdout),
            }
        return {"networks": dict(self.networks), "nodes": nodes}

    def setup(self, topology):
        self.nodes = {}
        self.networks = {}
        self.create_network("a", topology)
        self.create_node("NYC", "a", topology)
        if topology == "direct":
            self.create_node("SHA", "a", topology)
            source, target = self.address("NYC", "a"), self.address("SHA", "a")
        else:
            self.create_network("b", topology)
            self.create_node("LAX", "a", topology, forward=True)
            docker("network", "connect", self.networks["b"]["name"], self.nodes["LAX"])
            self.create_node("SHA", "b", topology)
            source, target = self.address("NYC", "a"), self.address("SHA", "b")
            self.exec("NYC", "ip", "route", "add", self.networks["b"]["subnet"], "via", self.address("LAX", "a"), "dev", self.interface("NYC", source))
            self.exec("SHA", "ip", "route", "add", self.networks["a"]["subnet"], "via", self.address("LAX", "b"), "dev", self.interface("SHA", target))
        self.remove_defaults()
        for node in self.nodes:
            interfaces = json.loads(self.exec(node, "ip", "-json", "link", "show").stdout)
            if any(interface["mtu"] != self.args.mtu for interface in interfaces if interface["ifname"] != "lo"):
                raise LabError(f"Docker did not apply the requested MTU {self.args.mtu} to {node}")
        wait_until("NYC to SHA health", lambda: self.probe("NYC", target))
        wait_until("SHA to NYC health", lambda: self.probe("SHA", source))
        proof = {
            "nyc_to_sha_ready": True, "sha_to_nyc_ready": True,
            "nyc_route": json.loads(self.exec("NYC", "ip", "-json", "route", "get", target).stdout),
            "sha_route": json.loads(self.exec("SHA", "ip", "-json", "route", "get", source).stdout),
        }
        if topology == "routed":
            # Docker mounts /proc/sys read-only even with NET_ADMIN. Bring the
            # router links down instead of requiring privileged sysctl writes.
            lax_interfaces = [self.interface("LAX", self.address("LAX", network)) for network in ("a", "b")]
            try:
                for interface in lax_interfaces:
                    self.exec("LAX", "ip", "link", "set", "dev", interface, "down")
                blocked_forward = not self.probe("NYC", target)
                blocked_reverse = not self.probe("SHA", source)
                if not blocked_forward or not blocked_reverse:
                    raise LabError("LAX links-down check failed: an endpoint can bypass LAX")
                proof["lax_links_down_blocks_both_directions"] = True
                proof["lax_interfaces_disabled_for_check"] = lax_interfaces
            finally:
                for interface in lax_interfaces:
                    self.exec("LAX", "ip", "link", "set", "dev", interface, "up")
            wait_until("LAX links restored NYC to SHA", lambda: self.probe("NYC", target))
            wait_until("LAX links restored SHA to NYC", lambda: self.probe("SHA", source))
        for node in self.nodes:
            if json.loads(self.exec(node, "ip", "-json", "route", "show", "default").stdout):
                raise LabError(f"Unexpected default route on {node}")
        return source, target, proof

    def cleanup(self):
        """Exact names plus ownership labels; never prune or touch unrelated resources."""
        errors = []
        for kind, names in (("container", self.containers_owned), ("network", self.networks_owned)):
            for name in reversed(names):
                try:
                    inspected = docker(kind, "inspect", name, check=False)
                    if inspected.returncode:
                        if "No such" in inspected.stderr or "not found" in inspected.stderr:
                            continue
                        raise LabError(inspected.stderr.strip())
                    info = json.loads(inspected.stdout)[0]
                    labels = info["Config"].get("Labels", {}) if kind == "container" else info.get("Labels", {})
                    if labels.get(RUN_LABEL) != self.prefix or labels.get(LABEL) != "transport":
                        raise LabError(f"Refusing to remove resource with different labels: {name}")
                    flags = ["--force"] if kind == "container" else []
                    docker(kind, "rm", *flags, name)
                except (LabError, ValueError) as error:
                    errors.append(str(error))
            names.clear()
        if self.owned_image:
            try:
                inspected = docker("image", "inspect", self.owned_image, check=False)
                if inspected.returncode == 0:
                    labels = json.loads(inspected.stdout)[0]["Config"].get("Labels", {})
                    if labels.get(RUN_LABEL) != self.prefix:
                        raise LabError("Refusing to remove image with different run label")
                    docker("image", "rm", self.owned_image)
            except (LabError, ValueError) as error:
                errors.append(str(error))
            self.owned_image = None
        return errors


def summarize(raw):
    end = raw.get("end", {})
    sender = end.get("sum_sent", {})
    receiver = end.get("sum_received", {})
    udp = end.get("sum", {})
    return {
        "sender_bits_per_second": sender.get("bits_per_second"),
        "receiver_bits_per_second": receiver.get("bits_per_second"),
        "retransmits": sender.get("retransmits"),
        "udp": {key: receiver.get(key, udp.get(key)) for key in ("jitter_ms", "lost_packets", "packets", "lost_percent")},
        "iperf_process_cpu_percent": end.get("cpu_utilization_percent"),
    }


def benchmark_case(fixture, source, target, protocol, parallel, reverse, iteration):
    args = fixture.args
    case_id = f"{protocol}-p{parallel}-{'reverse' if reverse else 'forward'}-{iteration}"
    server_file = f"/run/{case_id}.json"
    server = ["iperf3", "--server", "--one-off", "--bind", target, "--port", "5201", "--json", "--logfile", server_file]
    client = [
        "iperf3", "--client", target, "--bind", source, "--port", "5201", "--time", str(args.seconds),
        "--parallel", str(parallel), "--json", "--get-server-output", "--connect-timeout", "3000",
    ]
    if reverse:
        client.append("--reverse")
    omitted = getattr(args, "omit", 0)
    if omitted:
        client.extend(["--omit", str(omitted)])
    if protocol == "udp":
        client.extend(["--udp", "--bitrate", args.udp_rate, "--length", str(args.udp_length)])
    started = now()
    docker("exec", "--detach", fixture.nodes["SHA"], *server)
    wait_until("iperf3 listener", lambda: fixture.helper("SHA", "listening", "5201", check=False).returncode == 0)
    execution = fixture.exec("NYC", *client, timeout=args.seconds + omitted + 20, check=False)
    try:
        raw = json.loads(execution.stdout)
    except ValueError as error:
        raise LabError(f"Invalid iperf3 JSON: {execution.stdout}\n{execution.stderr}") from error
    case = {
        "id": case_id, "protocol": protocol, "parallel": parallel, "reverse": reverse,
        "iteration": iteration, "started_at": started, "finished_at": now(),
        "traffic_direction": "SHA->NYC" if reverse else "NYC->SHA",
        "control_direction": "NYC->SHA", "client_command": client, "server_command": server,
        "exit_code": execution.returncode, "stderr": execution.stderr, "raw_client": raw,
        "summary": summarize(raw),
    }
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
    case["raw_server"] = server_raw[0]
    case["server_iperf_process_cpu_percent"] = server_raw[0].get("end", {}).get("cpu_utilization_percent")
    case["status"] = "ok" if execution.returncode == 0 and not raw.get("error") and not server_raw[0].get("error") else "failed"
    return case


def bounded_integer(minimum, maximum):
    def parse(value):
        number = int(value)
        if not minimum <= number <= maximum:
            raise argparse.ArgumentTypeError(f"must be between {minimum} and {maximum}")
        return number
    return parse


def positive_cpus(value):
    number = float(value)
    if not math.isfinite(number) or not 0.1 <= number <= 8:
        raise argparse.ArgumentTypeError("must be between 0.1 and 8 CPUs")
    return number


def bounded_rate(value):
    match = re.fullmatch(r"([0-9]+(?:\.[0-9]+)?)([KMG]?)", value.upper())
    if not match:
        raise argparse.ArgumentTypeError("use a bounded iperf3 bitrate such as 20M")
    bits = float(match[1]) * {"": 1, "K": 1000, "M": 1000000, "G": 1000000000}[match[2]]
    if not 1000 <= bits <= 1000000000:
        raise argparse.ArgumentTypeError("UDP bitrate must be between 1K and 1G (never 0/unlimited)")
    return value.upper()


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new JSON output path (not an existing file)")
    parser.add_argument("--topology", choices=("both", "direct", "routed"), default="both")
    parser.add_argument("--seconds", type=bounded_integer(1, 30), default=3)
    parser.add_argument("--iterations", type=bounded_integer(1, 10), default=1)
    parser.add_argument("--udp-rate", type=bounded_rate, default="20M")
    parser.add_argument("--udp-length", type=bounded_integer(128, 1400), default=1200)
    parser.add_argument("--cpus", type=positive_cpus, default=1.0, help="CPU quota per node, default 1")
    parser.add_argument("--memory", choices=("128m", "256m", "512m", "1g"), default="256m")
    parser.add_argument("--mtu", type=bounded_integer(1280, 9000), default=1500, help="MTU for every bare-kernel bridge/interface, default 1500")
    parser.add_argument("--image", help="reuse a previously built lab image; architecture is checked")
    parser.add_argument("--base-image", default=BASE_IMAGE, help="base reference for automatic build, preferably digest-pinned")
    parser.add_argument("--build-timeout", type=bounded_integer(30, 1200), default=300)
    return parser.parse_args(argv)


def interrupted(signum, frame):
    raise KeyboardInterrupt(f"received signal {signum}")


def main(argv=None):
    args = arguments(argv)
    os.umask(0o077)
    output = args.output.expanduser().resolve()
    if output.exists():
        print(f"Refusing to overwrite existing output: {output}", file=sys.stderr)
        return 2
    if not output.parent.is_dir():
        print(f"Output parent must exist: {output.parent}", file=sys.stderr)
        return 2
    signal.signal(signal.SIGTERM, interrupted)
    run_id = f"meshway-lab-{datetime.now(timezone.utc).strftime('%Y%m%d%H%M%S')}-{uuid.uuid4().hex[:8]}"
    result = {
        "schema_version": 1, "run_id": run_id, "started_at": now(), "status": "running",
        "purpose": "pure iperf3 Linux Docker baseline; no Meshway data-plane benchmark",
        "location_semantics": "NYC/LAX/SHA are synthetic node names in one local Linux VM, not real regions",
        "settings": {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()},
        "measurement_notes": [
            "CPU percentages are iperf3 process accounting, not whole-container or kernel forwarding CPU.",
            "UDP uses a bounded offered bitrate; this is not a maximum-PPS benchmark.",
            "Docker exec is out-of-band control, with no shared management network or published host ports.",
            "Short local runs with CPU quotas are environment-specific baselines, not WAN measurements.",
        ],
        "topologies": [],
    }
    fixture = Fixture(args, result)
    exit_code = 1
    try:
        try:
            info = docker_json("info", "--format", "{{json .}}", timeout=15)
        except LabError as error:
            raise LabError(f"Linux Docker daemon unavailable. Start OrbStack/Docker and retry.\n{error}") from error
        if info.get("OSType") != "linux":
            raise LabError("A Linux Docker daemon is required")
        native_arch = architecture(info["Architecture"])
        host_arch = architecture(platform.machine())
        if native_arch != host_arch:
            raise LabError(f"Local native-only experiment: host {host_arch}, Docker {native_arch}; refusing possible emulation/remote architecture mismatch")
        result["environment"] = {
            "host": list(platform.uname()), "docker_context": docker("context", "show").stdout.strip(),
            "daemon": {key: info.get(key) for key in (
                "ServerVersion", "OperatingSystem", "OSType", "Architecture", "KernelVersion", "NCPU",
                "MemTotal", "CgroupDriver", "CgroupVersion", "CpuCfsPeriod", "CpuCfsQuota", "Warnings",
            )},
        }
        fixture.prepare_image(native_arch)
        write_result(output, result)
        for topology in (("direct", "routed") if args.topology == "both" else (args.topology,)):
            print(f"[{run_id}] Setting up {topology} baseline", flush=True)
            topology_result = {"name": topology, "cases": []}
            result["topologies"].append(topology_result)
            source, target, proof = fixture.setup(topology)
            topology_result["path_checks"] = proof
            topology_result["fixture"] = fixture.snapshot()
            write_result(output, result)
            for iteration in range(1, args.iterations + 1):
                for protocol, parallel in (("tcp", 1), ("tcp", 4), ("udp", 1)):
                    for reverse in (False, True):
                        case = benchmark_case(fixture, source, target, protocol, parallel, reverse, iteration)
                        topology_result["cases"].append(case)
                        write_result(output, result)
                        throughput = case["summary"]["receiver_bits_per_second"]
                        display = f"{throughput / 1e6:.2f} Mbit/s" if isinstance(throughput, (float, int)) else "unavailable"
                        print(f"  {case['id']}: {display} ({case['status']})", flush=True)
                        if case["status"] != "ok":
                            raise LabError(f"iperf3 failed: {case['id']}; see raw JSON")
            # Keep both fixture instances private and alive until final cleanup; at
            # most five bounded containers / three networks exist with --topology both.
        result["status"] = "ok"
        exit_code = 0
    except (LabError, OSError, ValueError, KeyError) as error:
        result["status"] = "failed"
        result["error"] = str(error)
        print(str(error), file=sys.stderr)
    except KeyboardInterrupt:
        result["status"] = "interrupted"
        result["error"] = "Interrupted; attempting owned-resource cleanup"
        exit_code = 130
    finally:
        # A second Ctrl-C must not strand the resources during the cleanup pass.
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        result["cleanup_errors"] = fixture.cleanup()
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
