#!/usr/bin/env python3
"""Explicit L2/L3 measurements in disposable Linux container namespaces."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import ipaddress
import json
import math
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from node_lab import check_elf, docker_target, image_builder, sha256


def command(*args, timeout=120, check=True):
    result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f"{args}: {result.stderr or result.stdout}")
    return result.stdout.strip()


def push(container, path, text):
    """Write a file into the read-only container's tmpfs; docker cp refuses read-only rootfs."""
    result = subprocess.run(["docker", "exec", "-i", container, "sh", "-c", f"cat > {path}"],
                            cwd=ROOT, input=text, text=True, capture_output=True, timeout=120)
    if result.returncode:
        raise RuntimeError(f"push {container}:{path}: {result.stderr or result.stdout}")


def source_digest():
    files = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "mise.toml", "mise.lock")]
    for directory in ("crates", "tools/meshway-dataplane-perf", "tools/meshway-static-authority", "scripts/perf", "dev/node-lab"):
        files.extend(p for p in (ROOT / directory).rglob("*")
                     if p.is_file() and "__pycache__" not in p.parts)
    files.append(ROOT / "scripts/node_lab.py")
    digest = hashlib.sha256()
    for path in sorted(files):
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def save(path, data):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(data, indent=2, allow_nan=False) + "\n")
    temporary.replace(path)


def last_profile(path):
    """Reports are cumulative: retain one final snapshot, never sum snapshots."""
    result = []
    for line in path.read_text().splitlines():
        event = json.loads(line)
        fields = event.get("fields", {})
        if "metrics_json" in fields:
            result = json.loads(fields["metrics_json"])
    return result


def receiver_metrics(data, traffic, payload):
    # iperf3 3.16's server UDP end.sum has zero bytes even after delivery.
    # Only sum_received is authoritative; retain the raw JSON beside this view.
    end = data["server_output_json"]["end"]
    received = end["sum_received"]
    if received["sender"] or received["seconds"] <= 0:
        raise ValueError("missing receiver measurement interval")
    if traffic == "udp":
        delivered = received["packets"] - received["lost_packets"]
        if delivered < 0 or received["bytes"] != delivered * payload:
            raise ValueError("receiver bytes and delivered UDP packets disagree")
    return received


def image_for(platform, output):
    builder = image_builder()
    digest = builder.source_digest()
    manifest = output / "image-manifest.json"
    if manifest.exists():
        cached = json.loads(manifest.read_text())
        if cached.get("status") == "built" and cached.get("sourceDigest") == digest and cached.get("platform") == platform:
            inspected = command("docker", "image", "inspect", cached["imageId"], check=False)
            if inspected:
                image = json.loads(inspected)[0]
                if (image["Os"] + "/" + image["Architecture"] == platform
                        and image["Config"].get("Labels", {}).get("io.meshway.node-lab.source") == digest):
                    return cached
    identifiers = command("docker", "image", "ls", "--no-trunc", "--quiet", "--filter",
                          "label=io.meshway.node-lab.source=" + digest).splitlines()
    for identifier in identifiers:
        image = json.loads(command("docker", "image", "inspect", identifier))[0]
        if image["Os"] + "/" + image["Architecture"] == platform:
            return {"imageId": image["Id"], "sourceDigest": digest,
                    "provenance": "existing immutable image of locked node-lab OS sources"}
    return builder.build(output, platform)


def summary(metadata):
    environment = {k: v for k, v in metadata["environment"].items() if k != "cpuInfo"}
    if "image" in environment:
        environment["image"] = {k: v for k, v in environment["image"].items()
                                if k in ("imageId", "sourceDigest", "base", "platform", "provenance")}
    lines = ["# L2/L3 TUN performance", "",
             "Plain = real TUN + Tokio + production UDP Session, without authentication, encryption, policy or Control.",
             "Medians of successful repeats; profiling=0 is the final throughput reference.",
             "Secure = real SecureNode + guarded carrier adapters + production UDP Session; relay has no TUN.",
             "wg-direct = kernel WireGuard A-B; wg-relay = A-R/R-B tunnels with R decrypting, routing and re-encrypting (not Meshway ciphertext relay semantics).",
             "WireGuard has no native mesh relay semantics. It uses kernel multicore workers/softirq and kernel offloads, versus Meshway current-thread userspace nodes.",
             "WireGuard data overhead is 32 bytes plus padding (excluding outer UDP/IP); Meshway's 139-byte example is IPv4/TCP, two hops, final segment, 7-byte listener keys (direct: 128 bytes). Actual overhead depends on protocol/path/key lengths.",
             "Raw/WireGuard batch=0 and profile=0 mean not applicable; each kernel reference runs once per traffic/repeat, independently of Meshway batch/profile variants.",
             "CPU is all sample containers' cgroup CPU seconds / full client wall interval ×100 (100%=one core); includes warmup.",
             "Kernel WireGuard crypto/softirq/kworker CPU may be charged outside these cgroups: total kernel CPU is unmeasured, not zero. Equal container quotas do not bound all kernel work; CPU deltas are cgroup accounting only.",
             "UDP pps is receiver delivered datagrams/s; TCP pps is unmeasured. Raw preserves kernel offloads.", "",
             "| Mode | Batch | Profile 1/N | Traffic | n | Mbps | UDP pps | Loss % | CPU % |",
             "| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |"]
    groups = {}
    for record in metadata["results"]:
        if record["status"] == "ok":
            groups.setdefault(tuple(record[k] for k in ("mode", "batch", "profile", "traffic")), []).append(record)
    for key, records in sorted(groups.items()):
        def med(field):
            values = [r[field] for r in records if r.get(field) is not None]
            return f"{statistics.median(values):.2f}" if values else "—"
        lines.append(f"| {key[0]} | {key[1]} | {key[2]} | {key[3]} | {len(records)} | "
                     f"{med('goodputMbps')} | {med('packetsPerSecond')} | {med('lostPercent')} | {med('cpuPercent')} |")
    lines.extend(["", "## L2 → L3 (same run, batch, profiling and traffic)", "",
                  "Deltas of medians; CPU and loss deltas are percentage points. Missing comparisons remain unmeasured.",
                  "| Mode | Batch | Profile 1/N | Traffic | Δ Mbps | Δ throughput % | Δ CPU pp | Δ loss pp |",
                  "| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |"])
    for key, records in sorted(groups.items()):
        if not key[0].startswith("secure-"):
            continue
        plain = groups.get(("plain", *key[1:]))
        if not plain:
            continue
        def median(rows, field):
            values = [r[field] for r in rows if r.get(field) is not None]
            return statistics.median(values) if values else None
        def delta(field):
            left, right = median(records, field), median(plain, field)
            return f"{left-right:+.2f}" if left is not None and right is not None else "—"
        base = median(plain, "goodputMbps")
        percent = f"{(median(records, 'goodputMbps')/base-1)*100:+.2f}" if base else "—"
        lines.append(f"| {key[0]} | {key[1]} | {key[2]} | {key[3]} | {delta('goodputMbps')} | {percent} | {delta('cpuPercent')} | {delta('lostPercent')} |")
    lines.extend(["", "## Meshway secure relative to WireGuard (same run and traffic)", "",
                  "Secure minus corresponding kernel reference medians (direct/direct, relay/relay); negative throughput means slower. CPU/loss are percentage points. Missing metrics remain unmeasured.",
                  "The relay comparison changes trust/encryption semantics. CPU subtraction is not a total crypto-cost comparison because kernel work can escape cgroup accounting and quota.",
                  "| Mode | WG reference | Batch | Profile 1/N | Traffic | Δ Mbps | Δ UDP pps | Δ loss pp | Δ cgroup CPU pp |",
                  "| --- | --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |"])
    for key, records in sorted(groups.items()):
        if not key[0].startswith("secure-"):
            continue
        reference = key[0].replace("secure-", "wg-", 1)
        baseline = groups.get((reference, 0, 0, key[3]), [])
        deltas = []
        for field in ("goodputMbps", "packetsPerSecond", "lostPercent", "cpuPercent"):
            left = [r[field] for r in records if r.get(field) is not None]
            right = [r[field] for r in baseline if r.get(field) is not None]
            deltas.append(f"{statistics.median(left)-statistics.median(right):+.2f}" if left and right else "—")
        lines.append(f"| {key[0]} | {reference} | {key[1]} | {key[2]} | {key[3]} | " + " | ".join(deltas) + " |")
    lines.extend(["", "## Per-role cgroup CPU", "",
                  "Same full client wall interval and cgroup scope as total CPU; includes iperf and warmup. Missing historical role counters are unmeasured.",
                  "| Mode | Batch | Profile 1/N | Traffic | A CPU % | R CPU % | B CPU % |",
                  "| --- | ---: | ---: | --- | ---: | ---: | ---: |"])
    for key, records in sorted(groups.items()):
        def role_cpu(role):
            values = [r["cpuByRole"][role]["cpuPercent"] for r in records
                      if role in r.get("cpuByRole", {})]
            return f"{statistics.median(values):.2f}" if values else "—"
        lines.append(f"| {key[0]} | {key[1]} | {key[2]} | {key[3]} | "
                     f"{role_cpu('a')} | {role_cpu('r')} | {role_cpu('b')} |")
    profiles = {}
    for record in metadata["results"]:
        if record["status"] != "ok":
            continue
        for metrics in record.get("profiles", {}).values():
            for metric in metrics:
                times = metric["times"]
                if times["self_cpu_ns"] is not None and times["self_cpu_samples"]:
                    total = profiles.setdefault((record["mode"], metric["name"]), [0, 0])
                    total[0] += times["self_cpu_samples"]
                    total[1] += times["self_cpu_ns"]
    if profiles:
        lines.extend(["", "## Sampled self CPU", "",
                      "Sum of each node's last cumulative snapshot across successful profiling-on samples; excludes unsampled work and any tail after the last report.",
                      "| Mode | Metric | CPU samples | Mean self CPU ns/sample | Sampled self CPU ms |",
                      "| --- | --- | ---: | ---: | ---: |"])
        for (mode, name), (count, ns) in sorted(profiles.items()):
            lines.append(f"| {mode} | {name} | {count} | {ns/count:.2f} | {ns/1e6:.2f} |")
    lines.extend(["", "## Guard calls per originating IP packet", "",
                  "Sum of exact control.guard calls / l3.packet.origin calls in each node's final cumulative profile snapshot after I/O shutdown. Includes setup, warmup, handshake/idle guards and output poll fences; not a fixed algorithmic count.",
                  "| Sample | Guard calls | Origin packets | Calls/packet |", "| --- | ---: | ---: | ---: |"])
    for record in metadata["results"]:
        calls = {}
        if record["status"] != "ok":
            continue
        for metrics in record.get("profiles", {}).values():
            for metric in metrics:
                calls[metric["name"]] = calls.get(metric["name"], 0) + metric["calls_started"]
        packets = calls.get("l3.packet.origin", 0)
        if packets:
            guards = calls.get("control.guard")
            value = f"{guards/packets:.2f}" if guards is not None else "—"
            lines.append(f"| {record['directory']} | {guards} | {packets} | {value} |")
    lines.extend(["", "Meshway recvmmsg/sendmmsg and UDP_SEGMENT/UDP_GRO: not implemented; event batches use per-packet syscalls. Raw/WireGuard preserve kernel defaults.",
                  f"Failed samples: {sum(r['status'] != 'ok' for r in metadata['results'])}; preserved in results.json, never replaced with zero.",
                  "", "## Provenance", "", "```json",
                  json.dumps({key: metadata.get(key) for key in ("revision", "dirty", "sourceDigest", "binarySha256", "fixtureBinarySha256", "options")}, indent=2), "```",
                  "", "## Environment", "", "```json",
                  json.dumps(environment, indent=2), "```", ""])
    return "\n".join(lines)


def cpu_usage(container):
    text = command("docker", "exec", container, "cat", "/sys/fs/cgroup/cpu.stat")
    return int(dict(line.split() for line in text.splitlines())["usage_usec"]) / 1_000_000


def namespace_options(mode, role):
    if mode.startswith("wg-"):
        return ["--cap-add", "NET_ADMIN"] + (["--sysctl", "net.ipv4.ip_forward=1"] if role == "r" else [])
    if role == "r":
        return []
    return ["--cap-add", "NET_ADMIN"] + ([] if mode == "raw" else ["--device", "/dev/net/tun:/dev/net/tun"])


def wireguard_setup(containers, addresses, mtu):
    """Kernel-only tunnels; private keys never leave each namespace's tmpfs."""
    relay = "r" in containers
    # Each entry is (role, interface, peer role, peer interface, port, peer port, AllowedIPs).
    topology = ([('a', 'wg0', 'r', 'wg0', 41000, 41000, '100.127.0.2/32'),
                 ('r', 'wg0', 'a', 'wg0', 41000, 41000, '100.127.0.1/32'),
                 ('r', 'wg1', 'b', 'wg0', 41001, 41000, '100.127.0.2/32'),
                 ('b', 'wg0', 'r', 'wg1', 41000, 41001, '100.127.0.1/32')] if relay else
                [('a', 'wg0', 'b', 'wg0', 41000, 41000, '100.127.0.2/32'),
                 ('b', 'wg0', 'a', 'wg0', 41000, 41000, '100.127.0.1/32')])
    public = {}
    for role, interface, *_ in topology:
        container = containers[role]
        command("docker", "exec", container, "sh", "-ec",
                f"umask 077; mkdir -p /tmp/private; wg genkey > /tmp/private/{interface}.key; "
                f"wg pubkey < /tmp/private/{interface}.key > /tmp/private/{interface}.pub")
        public[role, interface] = command("docker", "exec", container, "cat", f"/tmp/private/{interface}.pub")
        command("docker", "exec", container, "ip", "link", "add", interface, "type", "wireguard")
    evidence = {}
    for role, interface, peer, peer_interface, port, peer_port, allowed in topology:
        container = containers[role]
        # Public material is transported in memory, never included in reports or error command arguments.
        push(container, f"/tmp/private/{interface}.peer", public[peer, peer_interface] + "\n")
        endpoint = f"{addresses[peer]}:{peer_port}"
        command("docker", "exec", container, "sh", "-ec",
                f'wg set {interface} listen-port {port} private-key /tmp/private/{interface}.key '
                f'peer "$(cat /tmp/private/{interface}.peer)" allowed-ips {allowed} endpoint {endpoint}')
        if role != "r":
            local = "100.127.0.1/32" if role == "a" else "100.127.0.2/32"
            command("docker", "exec", container, "ip", "address", "add", local, "dev", interface)
        command("docker", "exec", container, "ip", "link", "set", interface, "mtu", str(mtu), "up")
        command("docker", "exec", container, "ip", "route", "add", allowed, "dev", interface)
        link = json.loads(command("docker", "exec", container, "ip", "-d", "-j", "link", "show", interface))[0]
        route = json.loads(command("docker", "exec", container, "ip", "-j", "route", "get", allowed.split('/')[0]))
        if link["linkinfo"]["info_kind"] != "wireguard" or link["mtu"] != mtu or route[0]["dev"] != interface:
            raise RuntimeError("WireGuard interface or route differs from the requested topology")
        evidence.setdefault(role, {})[interface] = dict(peer=peer, endpoint=endpoint, allowedIPs=allowed,
                                                       link=link, route=route)
    if relay and command("docker", "exec", containers["r"], "cat", "/proc/sys/net/ipv4/ip_forward") != "1":
        raise RuntimeError("WireGuard relay forwarding is disabled")
    return evidence


def wireguard_transfers(containers, evidence):
    """Validate every tunnel carried traffic; never dump WireGuard key material."""
    for role, interfaces in evidence.items():
        for interface, item in interfaces.items():
            values = command("docker", "exec", containers[role], "sh", "-ec",
                f"wg show {interface} latest-handshakes | awk '{{print $2}}'; "
                f"wg show {interface} transfer | awk '{{print $2, $3}}'").split()
            if len(values) != 3 or any(int(value) <= 0 for value in values):
                raise RuntimeError("WireGuard tunnel lacks handshake or bidirectional traffic")
            item.update(zip(("handshakeUnixSeconds", "receivedBytes", "sentBytes"), map(int, values)))


def sample(args, metadata, output, network, image, platform, identity, mode, batch, profile, traffic, repeat):
    label = f"{mode}-b{batch}-p{profile}-{traffic}-r{repeat}"
    print(label, flush=True)
    directory = output / label
    directory.mkdir()
    record = dict(mode=mode, batch=batch, profile=profile, traffic=traffic, repeat=repeat,
                  status="failed", directory=label, goodputMbps=None, packetsPerSecond=None,
                  lostPercent=None, cpuPercent=None, allocations=None, copiedBytes=None, peakMemoryBytes=None)
    containers = {}
    nodes = []
    server = None
    handles = []
    roles = ("a", "r", "b") if mode.endswith("-relay") else ("a", "b")
    secure = mode.startswith("secure-")
    wireguard = mode.startswith("wg-")
    try:
        for role in roles:
            name = f"meshway-dp-{identity}-{role}"
            container = command("docker", "create", "--name", name, "--label", "io.meshway.dataplane-perf=" + identity,
                                "--platform", platform, "--network", network,
                                 "--cap-drop", "ALL",
                                 *namespace_options(mode, role),
                                "--security-opt", "no-new-privileges", "--read-only", "--pids-limit", "128",
                                "--cpus", str(args.cpus), "--memory", args.memory,
                                "--tmpfs", "/tmp:rw,nosuid,nodev,mode=1777",
                                "--mount", f"type=bind,source={output / 'bin'},target=/fixtures,readonly",
                                "--entrypoint", "/bin/sleep", image, "infinity")
            containers[role] = container
            command("docker", "start", container)
        descriptions = {role: json.loads(command("docker", "inspect", container))[0]
                        for role, container in containers.items()}
        addresses = {role: item["NetworkSettings"]["Networks"][network]["IPAddress"]
                     for role, item in descriptions.items()}
        record["resources"] = {role: {key: item["HostConfig"][key] for key in ("NanoCpus", "Memory", "PidsLimit")}
                               for role, item in descriptions.items()}
        if not metadata["environment"].get("kernel"):
            metadata["environment"].update(kernel=command("docker", "exec", containers["a"], "uname", "-a"),
                iperf3=command("docker", "exec", containers["a"], "iperf3", "--version"),
                cpuInfo=command("docker", "exec", containers["a"], "cat", "/proc/cpuinfo"))
        if mode == "raw":
            for container in containers.values():
                command("docker", "exec", container, "ip", "link", "set", "eth0", "mtu", str(args.mtu))
        elif wireguard:
            metadata["environment"]["wireguardTools"] = command("docker", "exec", containers["a"], "wg", "--version")
            record["wireguard"] = wireguard_setup(containers, addresses, args.mtu)
        else:
            if secure:
                public_nodes = []
                for role in roles:
                    public_nodes.append(json.loads(command("docker", "exec", containers[role],
                        "/fixtures/meshway-dataplane-fixture", "material", "--directory", "/tmp/private",
                        "--socket", addresses[role] + ":41000")))
                save(directory / "public-nodes.json", public_nodes)
                push(containers["a"], "/tmp/public-nodes.json", (directory / "public-nodes.json").read_text())
                fixture = json.loads(command("docker", "exec", containers["a"], "/fixtures/meshway-dataplane-fixture",
                    "compile", "--input", "/tmp/public-nodes.json", "--seconds", str(args.seconds + args.omit + 120)))
                save(directory / "fixture.json", fixture)
                record["publicationDigest"] = fixture["publicationDigest"]
                for container in containers.values():
                    push(container, "/tmp/fixture.json", (directory / "fixture.json").read_text())
            for role in reversed(roles):
                other = "b" if role == "a" else "a"
                stdout = (directory / f"{role}.stdout").open("w")
                stderr = (directory / f"{role}.stderr").open("w")
                handles.extend([stdout, stderr])
                nodes.append(subprocess.Popen(["docker", "exec", "--env", f"MESHWAY_PROFILE={profile}",
                    "--env", "MESHWAY_PROFILE_DETAIL=1",
                    containers[role], "/fixtures/meshway-dataplane-perf", "--stage", "secure" if secure else "plain", "--role", role,
                    "--bind", addresses[role] + ":41000", "--peer", addresses[other] + ":41000",
                    "--mtu", str(args.mtu), "--batch", str(batch), "--seconds", str(args.seconds + args.omit + 60),
                    "--log-format", "json"] + (["--fixture", "/tmp/fixture.json", "--material", "/tmp/private/seeds.json"] if secure else []), cwd=ROOT, stdout=stdout, stderr=stderr))
            deadline = time.monotonic() + 15
            while not all((directory / f"{role}.stdout").read_text().strip() for role in roles):
                if any(node.poll() is not None for node in nodes) or time.monotonic() > deadline:
                    raise RuntimeError("node failed readiness; see process logs")
                time.sleep(0.05)
            for role in roles:
                readiness = json.loads((directory / f"{role}.stdout").read_text())
                if not readiness["ready"]:
                    raise RuntimeError("invalid node readiness")
                if secure and readiness["publicationDigest"] != record["publicationDigest"]:
                    raise RuntimeError("node compiled publication digest differs")
                if role == "r" and readiness["tun"] is not None:
                    raise RuntimeError("relay must not open TUN")
        destination = addresses["b"] if mode == "raw" else "100.127.0.2"
        record["route"] = json.loads(command("docker", "exec", containers["a"], "ip", "-j", "route", "get", destination))
        record["links"] = {role: json.loads(command("docker", "exec", container, "ip", "-j", "link"))
                           for role, container in containers.items()}
        expected_device = ("eth0" if mode == "raw" else "wg0" if wireguard else
                           json.loads((directory / "a.stdout").read_text())["tun"])
        if record["route"][0]["dev"] != expected_device:
            raise RuntimeError("iperf destination is not routed through the expected interface")
        server_out = (directory / "iperf-server.json").open("w")
        server_err = (directory / "iperf-server.stderr").open("w")
        handles.extend([server_out, server_err])
        server = subprocess.Popen(["docker", "exec", containers["b"], "iperf3", "-s", "-1", "-J"],
                                  cwd=ROOT, stdout=server_out, stderr=server_err)
        deadline = time.monotonic() + 5
        while ":1451" not in command("docker", "exec", containers["b"], "cat", "/proc/net/tcp", "/proc/net/tcp6"):
            if server.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("iperf server failed readiness")
            time.sleep(0.05)
        before = {role: cpu_usage(c) for role, c in containers.items()}
        start = time.monotonic()
        client_args = ["docker", "exec", containers["a"], "iperf3", "-c", destination, "-J",
                       "-t", str(args.seconds), "-O", str(args.omit), "-P", "1", "--get-server-output"]
        if traffic == "udp":
            client_args.extend(["-u", "-b", args.udp_rate, "-l", str(args.payload)])
        with (directory / "iperf-client.json").open("w") as stdout, (directory / "iperf-client.stderr").open("w") as stderr:
            completed = subprocess.run(client_args, cwd=ROOT, stdout=stdout, stderr=stderr, timeout=args.seconds + args.omit + 25)
        elapsed = time.monotonic() - start
        after = {role: cpu_usage(c) for role, c in containers.items()}
        cpu = {role: after[role] - before[role] for role in containers}
        if any(not math.isfinite(value) or value < 0 for value in cpu.values()):
            raise RuntimeError("invalid per-role cgroup CPU delta")
        total_cpu = sum(cpu.values())
        record.update(cpuSeconds=total_cpu, clientWallSeconds=elapsed, command=client_args,
                      cpuByRole={role: {"cpuSeconds": value, "cpuPercent": value/elapsed*100}
                                 for role, value in cpu.items()})
        if completed.returncode or server.wait(timeout=10):
            raise RuntimeError("iperf process failed")
        data = json.loads((directory / "iperf-client.json").read_text())
        if "error" in data:
            raise RuntimeError(data["error"])
        received = receiver_metrics(data, traffic, args.payload)
        record.update(status="ok", goodputMbps=received["bits_per_second"]/1e6,
                       cpuPercent=total_cpu/elapsed*100,
                      packetsPerSecond=((received["packets"]-received["lost_packets"])/received["seconds"] if traffic == "udp" else None),
                      lostPercent=received.get("lost_percent"))
        for field in ("goodputMbps", "cpuPercent", "packetsPerSecond", "lostPercent"):
            value = record[field]
            if value is not None and (not math.isfinite(value) or value < 0):
                record[field] = None
                raise RuntimeError("invalid receiver metric: " + field)
        if nodes and any(node.poll() is not None for node in nodes):
            raise RuntimeError("plain node exited during measurement")
        if wireguard:
            wireguard_transfers(containers, record["wireguard"])
    except (RuntimeError, subprocess.TimeoutExpired, KeyError, ValueError) as error:
        record.update(status="failed", failure=str(error))
    finally:
        for container in containers.values():
            # Only fixture-owned process names in these newly created namespaces.
            command("docker", "exec", container, "pkill", "-INT", "-f", "^/fixtures/meshway-dataplane-perf", check=False)
        for node in nodes:
            try:
                node.wait(timeout=10)
            except subprocess.TimeoutExpired:
                record.update(status="failed", failure="plain node did not shut down")
        for container in containers.values():
            command("docker", "rm", "-f", container)
        if server is not None:
            server.wait(timeout=10)
        for node in nodes:
            node.wait(timeout=10)
            if node.returncode:
                record.update(status="failed", failure=f"plain node exit {node.returncode}; see stderr")
        for handle in handles:
            handle.close()
    record["nodeExitCodes"] = [node.returncode for node in nodes]
    if profile:
        try:
            record["profiles"] = {role: last_profile(directory / f"{role}.stderr") for role in roles
                                  if (directory / f"{role}.stderr").exists()}
        except (ValueError, TypeError) as error:
            record.update(status="failed", failure="invalid profile output: " + str(error))
    return record


def measure(args, output_root):
    if shutil.disk_usage(ROOT).free < 50 * 1024**3:
        raise SystemExit("Performance build needs 50 GiB free disk.")
    identity = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ").lower() + "-" + uuid.uuid4().hex[:8]
    output = output_root / identity
    (output / "bin").mkdir(parents=True)
    target, platform, machine = docker_target()
    before = source_digest()
    cache = ROOT / "target/dataplane-perf"
    subprocess.run(["rustup", "target", "add", target], cwd=ROOT, check=True)
    subprocess.run(["cargo", "zigbuild", "--release", "--locked", "--target", target, "-p", "meshway-dataplane-perf"],
                   cwd=ROOT, check=True, env=dict(os.environ, CARGO_TARGET_DIR=str(cache), CARGO_INCREMENTAL="0"))
    if source_digest() != before:
        raise RuntimeError("sources changed during build")
    binary = output / "bin/meshway-dataplane-perf"
    shutil.copy2(cache / target / "release/meshway-dataplane-perf", binary)
    binary.chmod(0o500)
    check_elf(binary, machine)
    helper = output / "bin/meshway-dataplane-fixture"
    shutil.copy2(cache / target / "release/meshway-dataplane-fixture", helper)
    helper.chmod(0o500)
    check_elf(helper, machine)
    image = image_for(platform, output_root / "image")
    metadata = dict(schemaVersion=1, revision=command("git", "rev-parse", "HEAD"),
                    dirty=bool(command("git", "status", "--porcelain")), sourceDigest=before,
                    binarySha256=sha256(binary), fixtureBinarySha256=sha256(helper), options=vars(args), results=[],
                    environment=dict(target=target, platform=platform, image=image,
                        rustc=command("rustc", "-Vv"), zig=command("zig", "version"),
                        cargoZigbuild=command("cargo-zigbuild", "--version"),
                        cpusPerContainer=args.cpus, memoryPerContainer=args.memory,
                        runtime="Meshway: Tokio current_thread per node; WireGuard: kernel workers/softirq", mtu=args.mtu, udpPayloadBytes=args.payload,
                        profileDetail=1,
                        udpRate=args.udp_rate, warmupOmittedSeconds=args.omit,
                        cpuScope="all sample cgroups over full client wall interval including warmup (relay: 3, others: 2)",
                        kernelCpuLimitation="WireGuard crypto/softirq/kworker CPU may be charged outside sample cgroups; total kernel CPU is unmeasured, not zero; quotas do not bound all kernel work",
                        wireguardRelay="A-R and R-B tunnels; R decrypts, routes and re-encrypts, unlike Meshway ciphertext relay",
                        offloadScope="kernel defaults for raw/WireGuard; following flags describe Meshway userspace only",
                        kernelBatch=False, udpSegment=False, udpGro=False))
    save(output / "results.json", metadata)
    network = "meshway-dp-" + identity
    command("docker", "network", "create", "--internal", "--label", "io.meshway.dataplane-perf=" + identity, network)
    try:
        description = json.loads(command("docker", "network", "inspect", network))[0]
        for config in description["IPAM"]["Config"]:
            subnet = ipaddress.ip_network(config["Subnet"])
            if subnet.version == 4 and subnet.overlaps(ipaddress.ip_network("100.127.0.0/30")):
                raise RuntimeError("fixture underlay overlaps the TUN subnet")
        metadata["environment"]["network"] = description["IPAM"]["Config"]
        configs = [(mode, 0, 0) for mode in ("raw", "wg-direct", "wg-relay")] + [(mode, b, p) for mode in ("plain", "secure-direct", "secure-relay")
                                   for b in args.batch for p in args.profile]
        for repeat in range(args.repeats):
            for mode, batch, profile in (configs if repeat % 2 == 0 else list(reversed(configs))):
                for traffic in ("tcp", "udp"):
                    metadata["results"].append(sample(args, metadata, output, network, image["imageId"], platform,
                        identity, mode, batch, profile, traffic, repeat))
                    save(output / "results.json", metadata)
                    (output / "summary.md").write_text(summary(metadata))
        metadata["sourcesUnchanged"] = source_digest() == before
        metadata["passed"] = metadata["sourcesUnchanged"] and all(r["status"] == "ok" for r in metadata["results"])
        save(output / "results.json", metadata)
        (output / "summary.md").write_text(summary(metadata))
    finally:
        command("docker", "network", "rm", network)
    print(output)
    return int(not metadata.get("sourcesUnchanged") or any(r["status"] != "ok" for r in metadata["results"]))


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--batch", type=int, nargs="+", default=[1, 32])
    parser.add_argument("--profile", type=int, nargs="+", default=[0, 128])
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--seconds", type=int, default=8)
    parser.add_argument("--omit", type=int, default=2)
    parser.add_argument("--mtu", type=int, default=1400)
    parser.add_argument("--payload", type=int, default=1200)
    parser.add_argument("--udp-rate", default="500M")
    parser.add_argument("--cpus", type=float, default=2)
    parser.add_argument("--memory", default="512m")
    args = parser.parse_args()
    if (not 2 <= args.repeats <= 100 or not 6 <= args.seconds <= 300 or not 1 <= args.omit <= 60
            or not 1280 <= args.mtu <= 1472 or not 16 <= args.payload <= args.mtu-28
            or any(not 1 <= b <= 1024 for b in args.batch) or any(not 0 <= p <= 65536 for p in args.profile)
            or not 0 < args.cpus <= 32):
        parser.error("measurement bounds invalid (at least 2 repeats, 6s measurement, 1s omitted warmup)")
    output_root = ROOT / ".meshway/perf/dataplane"
    output_root.mkdir(parents=True, exist_ok=True)
    with (output_root / "measurement.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return measure(args, output_root)


if __name__ == "__main__":
    sys.exit(main())
