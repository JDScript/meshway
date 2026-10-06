#!/usr/bin/env python3
"""Explicit L2 measurements in two disposable Linux container namespaces."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import ipaddress
import json
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


def source_digest():
    files = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "mise.toml", "mise.lock")]
    for directory in ("crates", "tools/meshway-dataplane-perf", "scripts/perf", "dev/node-lab"):
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


def image_for(platform, output):
    builder = image_builder()
    digest = builder.source_digest()
    identifiers = command("docker", "image", "ls", "--no-trunc", "--quiet", "--filter",
                          "label=io.meshway.node-lab.source=" + digest).splitlines()
    for identifier in identifiers:
        image = json.loads(command("docker", "image", "inspect", identifier))[0]
        if image["Os"] + "/" + image["Architecture"] == platform:
            return {"imageId": image["Id"], "sourceDigest": digest,
                    "provenance": "existing immutable image of locked node-lab OS sources"}
    return builder.build(output, platform)


def summary(metadata):
    lines = ["# L2 plain TUN performance", "",
             "Plain = real TUN + Tokio + production UDP Session, without authentication, encryption, policy or Control.",
             "Medians of successful repeats; profiling=0 is the final throughput reference.",
             "CPU is both containers' cgroup CPU seconds / full client wall interval ×100 (100%=one core); includes warmup.",
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
    lines.extend(["", "recvmmsg/sendmmsg and UDP_SEGMENT/UDP_GRO: not implemented; event batches use per-packet syscalls.",
                  f"Failed samples: {sum(r['status'] != 'ok' for r in metadata['results'])}; preserved in results.json, never replaced with zero.",
                  "", "## Environment", "", "```json",
                  json.dumps({k: v for k, v in metadata["environment"].items() if k != "cpuInfo"}, indent=2), "```", ""])
    return "\n".join(lines)


def cpu_usage(container):
    text = command("docker", "exec", container, "cat", "/sys/fs/cgroup/cpu.stat")
    return int(dict(line.split() for line in text.splitlines())["usage_usec"]) / 1_000_000


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
    try:
        for role in ("a", "b"):
            name = f"meshway-dp-{identity}-{role}"
            container = command("docker", "create", "--name", name, "--label", "io.meshway.dataplane-perf=" + identity,
                                "--platform", platform, "--network", network,
                                "--cap-drop", "ALL", "--cap-add", "NET_ADMIN", "--device", "/dev/net/tun:/dev/net/tun",
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
        else:
            for role, other in (("a", "b"), ("b", "a")):
                stdout = (directory / f"{role}.stdout").open("w")
                stderr = (directory / f"{role}.stderr").open("w")
                handles.extend([stdout, stderr])
                nodes.append(subprocess.Popen(["docker", "exec", "--env", f"MESHWAY_PROFILE={profile}",
                    containers[role], "/fixtures/meshway-dataplane-perf", "--stage", "plain", "--role", role,
                    "--bind", addresses[role] + ":41000", "--peer", addresses[other] + ":41000",
                    "--mtu", str(args.mtu), "--batch", str(batch), "--seconds", str(args.seconds + args.omit + 60),
                    "--log-format", "json"], cwd=ROOT, stdout=stdout, stderr=stderr))
            deadline = time.monotonic() + 15
            while not all((directory / f"{role}.stdout").read_text().strip() for role in ("a", "b")):
                if any(node.poll() is not None for node in nodes) or time.monotonic() > deadline:
                    raise RuntimeError("plain node failed readiness; see process logs")
                time.sleep(0.05)
            for role in ("a", "b"):
                if not json.loads((directory / f"{role}.stdout").read_text())["ready"]:
                    raise RuntimeError("invalid node readiness")
        destination = addresses["b"] if mode == "raw" else "100.127.0.2"
        record["route"] = json.loads(command("docker", "exec", containers["a"], "ip", "-j", "route", "get", destination))
        record["links"] = {role: json.loads(command("docker", "exec", container, "ip", "-j", "link"))
                           for role, container in containers.items()}
        expected_device = "eth0" if mode == "raw" else json.loads((directory / "a.stdout").read_text())["tun"]
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
        before = sum(cpu_usage(c) for c in containers.values())
        start = time.monotonic()
        client_args = ["docker", "exec", containers["a"], "iperf3", "-c", destination, "-J",
                       "-t", str(args.seconds), "-O", str(args.omit), "-P", "1", "--get-server-output"]
        if traffic == "udp":
            client_args.extend(["-u", "-b", args.udp_rate, "-l", str(args.payload)])
        with (directory / "iperf-client.json").open("w") as stdout, (directory / "iperf-client.stderr").open("w") as stderr:
            completed = subprocess.run(client_args, cwd=ROOT, stdout=stdout, stderr=stderr, timeout=args.seconds + args.omit + 25)
        elapsed = time.monotonic() - start
        after = sum(cpu_usage(c) for c in containers.values())
        record.update(cpuSeconds=after-before, clientWallSeconds=elapsed, command=client_args)
        if completed.returncode or server.wait(timeout=10):
            raise RuntimeError("iperf process failed")
        data = json.loads((directory / "iperf-client.json").read_text())
        if "error" in data:
            raise RuntimeError(data["error"])
        received = data["end"]["sum_received"] if traffic == "tcp" else data["end"]["sum"]
        # UDP end.sum is receiver-reported delivery/loss in this locked iperf3.
        if traffic == "udp":
            received = data["server_output_json"]["end"]["sum"]
        record.update(status="ok", goodputMbps=received["bits_per_second"]/1e6,
                      cpuPercent=(after-before)/elapsed*100,
                      packetsPerSecond=((received["packets"]-received["lost_packets"])/received["seconds"] if traffic == "udp" else None),
                      lostPercent=received.get("lost_percent"))
        if nodes and any(node.poll() is not None for node in nodes):
            raise RuntimeError("plain node exited during measurement")
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
    image = image_for(platform, output_root / "image")
    metadata = dict(schemaVersion=1, revision=command("git", "rev-parse", "HEAD"),
                    dirty=bool(command("git", "status", "--porcelain")), sourceDigest=before,
                    binarySha256=sha256(binary), options=vars(args), results=[],
                    environment=dict(target=target, platform=platform, image=image,
                        rustc=command("rustc", "-Vv"), zig=command("zig", "version"),
                        cargoZigbuild=command("cargo", "zigbuild", "--version"),
                        cpusPerContainer=args.cpus, memoryPerContainer=args.memory,
                        runtime="Tokio current_thread per node", mtu=args.mtu, udpPayloadBytes=args.payload,
                        udpRate=args.udp_rate, warmupOmittedSeconds=args.omit,
                        cpuScope="both cgroups over full client wall interval including warmup",
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
        configs = [("raw", 0, 0)] + [("plain", b, p) for b in args.batch for p in args.profile]
        for repeat in range(args.repeats):
            for mode, batch, profile in (configs if repeat % 2 == 0 else list(reversed(configs))):
                for traffic in ("tcp", "udp"):
                    metadata["results"].append(sample(args, metadata, output, network, image["imageId"], platform,
                        identity, mode, batch, profile, traffic, repeat))
                    save(output / "results.json", metadata)
                    (output / "summary.md").write_text(summary(metadata))
        metadata["sourcesUnchanged"] = source_digest() == before
        save(output / "results.json", metadata)
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
