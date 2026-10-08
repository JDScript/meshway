#!/usr/bin/env python3
"""Real meshwayd/CLI Linux conformance; host-only build, disposable namespaces.

Builds once on the host, then runs precompiled actors in isolated containers.
"""
import argparse
import concurrent.futures
import ipaddress
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid

from node_lab import ROOT, check_elf, docker_target, lab_dir, run, selected_image, sha256, verify_image
from verification_sources import fingerprint as source_fingerprint
from validation_support import build_lock, report_directory, bind_access_options, export_evidence, mount_namespace_options, link_artifact

BINARIES = ("meshwayd", "meshway", "meshway-runtime-fixture")
KERNEL_BINARY = "meshway-underlay-kernel-tests"
BOOTSTRAP_BINARY = "meshway-underlay-bootstrap-tests"
LIFECYCLE_BINARY = "meshway-daemon-lifecycle-tests"
LIFECYCLE_TEST = "cleanup::blocked_down_retries_real_dns_reservation_cleanup_without_going_online"
OWNER_CONFLICT_TEST = "runtime::control::tests::live_tun_owner_conflict_retries_and_recovers_without_new_up"
PLATFORM_BINARIES = {"platform-dns": ("meshway_dns", "meshway-dns-platform-tests"),
                     "platform-daemon": ("meshway_daemon", "meshway-daemon-platform-tests")}
ARTIFACTS = (*BINARIES, KERNEL_BINARY, BOOTSTRAP_BINARY, LIFECYCLE_BINARY, *(binary for _, binary in PLATFORM_BINARIES.values()))
ROLES = ("controller", "a", "r", "b")
KERNEL_TESTS = {
    "underlay-interface-multicast": "linux::tests::userspace_multicast_cannot_poison_kernel_monitor_or_control_proxy",
    "underlay-journal-classification": "linux::kernel::journal::tests::journal_classifies_same_namespace_changed_namespace_and_changed_boot",
    "underlay-journal-archive": "linux::kernel::journal::tests::changed_namespace_archives_exact_journal_and_same_namespace_recovers",
    "underlay-journal-boot": "linux::kernel::journal::tests::previous_boot_journal_is_not_a_conflict_or_a_recovery",
    "underlay-restart-a": "linux::kernel::tests::restart::container_restart_archives_previous_namespace_journal",
    "underlay-restart-b": "linux::kernel::tests::restart::container_restart_archives_previous_namespace_journal",
    "underlay-negative": "linux::kernel::tests::negative_capture_marked_bypass_recovery_and_exact_down",
    "underlay-foreign": "linux::kernel::tests::foreign_same_priority_is_retained_and_blocks_cleanup",
    "underlay-datagram": "linux::datagram::tests::actual_udp_mark_truncation_and_unpolled_owner_release",
    "underlay-dns-anchor": "linux::kernel::tests::dns_anchor_survives_suspend_and_owner_restart_until_explicit_removal",
    "underlay-dns-foreign": "linux::kernel::tests::foreign_dns_anchor_mutation_blocks_cleanup_without_removing_the_interface",
    "underlay-local-conflict": "linux::kernel::capture::tests::local_route_preflight_and_live_address_conflicts_preserve_foreign_resources",
}


def fingerprint():
    return source_fingerprint(ROOT)


def record_load():
    """Record host load for the report; functional conformance has no idle gate.
    The matrix runner owns parallelism and is recorded when present."""
    load = os.getloadavg()
    uptime = run("uptime", capture=True).strip()
    print(uptime, flush=True)
    record = {"uptime": uptime, "loadAverage": list(load), "waitSeconds": 0,
              "relaxedAfterTenMinutes": False}
    if os.environ.get("MESHWAY_MATRIX_RUN"):
        record["matrixRun"] = os.environ["MESHWAY_MATRIX_RUN"]
    return record


def write_json(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def build(directory, target, platform, machine, source):
    if shutil.disk_usage(ROOT).free < 50 * 1024 ** 3:
        raise RuntimeError("Linux host build needs at least 50 GiB free.")
    env = dict(os.environ, CARGO_INCREMENTAL="0",
               CARGO_TARGET_DIR=str(ROOT / "target/node-kernel-tests"))
    run("rustup", "target", "add", target, env=env)
    command = ["cargo", "zigbuild", "--target", target, "--locked",
                "--jobs", "2", "--all-features", "-p", "meshway-daemon", "-p", "meshway-cli",
                "-p", "meshway-runtime-fixture", "-p", "meshway-underlay", "-p", "meshway-dns", "--all-targets", "--message-format=json"]
    try:
        output = run(*command, capture=True, env=env)
    except subprocess.CalledProcessError as error:
        output = error.stdout or ""
        (directory / "build.jsonl").write_text(output)
        for line in output.splitlines():
            if line.startswith("{"):
                message = json.loads(line)
                if message.get("reason") == "compiler-message":
                    print(message["message"].get("rendered", ""), file=sys.stderr)
        raise
    (directory / "build.jsonl").write_text(output)
    if fingerprint() != source:
        raise RuntimeError("Sources changed during the shared daemon/CLI/fixture build.")
    hashes = {}
    for name in BINARIES:
        binary = directory / name
        shutil.copyfile(Path(env["CARGO_TARGET_DIR"]) / target / "debug" / name, binary)
        binary.chmod(0o500)
        check_elf(binary, machine)
        hashes[name] = sha256(binary)
    messages = [json.loads(line) for line in output.splitlines() if line.startswith("{")]
    executables = [Path(item["executable"]) for item in messages
                   if item.get("reason") == "compiler-artifact" and item.get("executable")
                   and item["target"]["name"] == "meshway_underlay" and item["target"]["kind"] == ["lib"]
                   and item["profile"].get("test")]
    if len(executables) != 1 or fingerprint() != source:
        raise RuntimeError("Expected one same-source production underlay library test executable.")
    check_elf(executables[0], machine)
    shutil.copyfile(executables[0], directory / KERNEL_BINARY)
    (directory / KERNEL_BINARY).chmod(0o500)
    hashes[KERNEL_BINARY] = sha256(directory / KERNEL_BINARY)
    bootstrap = [Path(item["executable"]) for item in messages
                 if item.get("reason") == "compiler-artifact" and item.get("executable")
                 and item["target"]["name"] == "bootstrap" and item["profile"].get("test")]
    if len(bootstrap) != 1:
        raise RuntimeError("Expected one production HTTPS bootstrap test executable.")
    check_elf(bootstrap[0], machine)
    shutil.copyfile(bootstrap[0], directory / BOOTSTRAP_BINARY)
    (directory / BOOTSTRAP_BINARY).chmod(0o500)
    hashes[BOOTSTRAP_BINARY] = sha256(directory / BOOTSTRAP_BINARY)
    lifecycle = [Path(item["executable"]) for item in messages
                 if item.get("reason") == "compiler-artifact" and item.get("executable")
                 and item["target"]["name"] == "lifecycle" and item["profile"].get("test")]
    if len(lifecycle) != 1:
        raise RuntimeError("Expected one production daemon lifecycle test executable.")
    check_elf(lifecycle[0], machine)
    shutil.copyfile(lifecycle[0], directory / LIFECYCLE_BINARY)
    (directory / LIFECYCLE_BINARY).chmod(0o500)
    hashes[LIFECYCLE_BINARY] = sha256(directory / LIFECYCLE_BINARY)
    for library, name in PLATFORM_BINARIES.values():
        tests = [Path(item["executable"]) for item in messages
                 if item.get("reason") == "compiler-artifact" and item.get("executable")
                 and item["target"]["name"] == library and item["target"]["kind"] == ["lib"]
                 and item["profile"].get("test")]
        if len(tests) != 1:
            raise RuntimeError("Expected one same-source Linux library test executable: " + library)
        check_elf(tests[0], machine)
        shutil.copyfile(tests[0], directory / name)
        (directory / name).chmod(0o500)
        hashes[name] = sha256(directory / name)
    manifest = {"sourceDigest": source, "sourceRoot": str(ROOT.resolve()), "target": target, "platform": platform,
                 "sha256": hashes, "buildCommands": [command],
                "revision": run("git", "rev-parse", "HEAD", capture=True).strip()}
    write_json(directory / "source-manifest.json", manifest)
    return manifest


def import_artifacts(source_dir, directory, target, platform, machine, source):
    manifest = json.loads((source_dir / "source-manifest.json").read_text())
    if (manifest["sourceDigest"] != source or manifest["sourceRoot"] != str(ROOT.resolve()) or manifest["target"] != target
            or manifest["platform"] != platform or set(manifest["sha256"]) != set(ARTIFACTS)):
        raise RuntimeError("Prebuilt manifest does not match this source/target/binary set.")
    for name in ARTIFACTS:
        binary = source_dir / name
        check_elf(binary, machine)
        if sha256(binary) != manifest["sha256"][name]:
            raise RuntimeError("Prebuilt binary does not match its source manifest.")
        link_artifact(binary, directory / name)
    write_json(directory / "source-manifest.json", manifest)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-only", action="store_true", help="host build and source manifest only")
    parser.add_argument("--report-dir", type=Path, help="new private report directory")
    parser.add_argument("--short", action="store_true", help="30-second diagnostic traffic; not acceptance")
    parser.add_argument("--runtime-image-manifest", type=Path, help="explicit locked node-lab OS image")
    parser.add_argument("--artifacts", type=Path, help="reuse a --build-only bin directory with matching source manifest")
    parser.add_argument("--transport", choices=("native-tcp", "native-udp", "vless", "trojan"), default="native-tcp",
                        help="Server-configured outer carrier; never a daemon-local policy override")
    parser.add_argument("--dns-backend", choices=("off", "resolv-conf", "resolved"), default="off",
                        help="real libc DNS conformance in an owned private mount namespace")
    parser.add_argument("--dns-image-manifest", type=Path,
                        help="parent-built dev/dns-fixture/build.py immutable OS image manifest")
    parser.add_argument("--traversal", choices=("off", "nat"), default="off",
                        help="real two-router IPv4 NAT acquisition with empty UDP endpoints")
    parser.add_argument("--traversal-fallback", action="store_true",
                         help="preauthorize TCP on R, then block only A-R UDP during the same 125-second flows")
    parser.add_argument("--reported-endpoints", action="store_true",
                        help="UDP/off without configured endpoints or STUN; use actual Node interface reports")
    parser.add_argument("--traversal-image-manifest", type=Path,
                        help="immutable OS manifest built with dev/dns-fixture/build.py --lock-file dev/traversal-fixture/assets.lock.json")
    args = parser.parse_args()
    if args.reported_endpoints and (args.transport != "native-udp" or args.traversal != "off" or args.dns_backend != "off"):
        parser.error("--reported-endpoints requires native-udp with traversal and DNS off")
    if not args.build_only and (args.dns_backend != "off") != bool(args.dns_image_manifest):
        parser.error("DNS scenarios require --dns-image-manifest; IP-only runs must omit it")
    if args.traversal == "nat" and args.transport != "native-udp":
        parser.error("--traversal nat requires --transport native-udp")
    if args.traversal_fallback and args.traversal != "nat":
        parser.error("--traversal-fallback requires --traversal nat")
    if not args.build_only and (args.traversal == "nat") != bool(args.traversal_image_manifest):
        parser.error("NAT runs require --traversal-image-manifest; static runs must omit it")
    os.umask(0o077)
    target, platform, machine = docker_target()  # read-only engine architecture query
    report = report_directory("runtime", args.report_dir)
    identity = report.name
    for name in ("bin", "public", "public/empty-cert-dir", "controller", "signals", "dns-support", "traversal-support", "router-a", "router-r", *ROLES,
                  "underlay-peer", "bootstrap", "daemon-cleanup", "daemon-owner-conflict", "restart", *KERNEL_TESTS, *PLATFORM_BINARIES):
        (report / name).mkdir(parents=True, exist_ok=True)
    source = fingerprint()
    # One source/cache owner per invocation; never leak the lock FD to Cargo,
    # Docker attach processes, or any later runtime child.
    if args.artifacts:
        artifacts = import_artifacts(args.artifacts.resolve(), report / "bin", target, platform, machine, source)
    else:
        with build_lock():
            artifacts = build(report / "bin", target, platform, machine, source)
    if args.build_only:
        print(report / "bin")
        return
    load = record_load()
    runtime_started = time.monotonic()
    # Full runtime actors invoke iproute2 in addition to the Rust binaries.
    # Use the explicitly built, provenance-checked Ubuntu tools image, never
    # install packages or launch its persistent-lab supervisor as a side effect.
    runtime_os = (verify_image(json.loads(args.runtime_image_manifest.read_text()), platform)
                  if args.runtime_image_manifest else selected_image(lab_dir(), platform))
    image = runtime_os["imageId"]
    description = json.loads(run("docker", "image", "inspect", image, capture=True))[0]
    if description["Os"] != "linux" or description["Architecture"] != platform.split("/")[1] or description["Config"].get("Volumes"):
        raise RuntimeError("Expected the existing native Linux digest-pinned runtime image without volumes.")
    dns_image = None
    dns_assets = None
    if args.dns_backend != "off":
        dns_assets = json.loads(args.dns_image_manifest.read_text())
        lock_path = ROOT / "dev/dns-fixture/assets.lock.json"
        lock = json.loads(lock_path.read_text())
        if (dns_assets.get("status") != "built" or dns_assets.get("platform") != platform
                or dns_assets.get("base") != lock["base"] or dns_assets.get("lockSha256") != sha256(lock_path)
                or not dns_assets.get("packages")):
            raise RuntimeError("DNS OS asset manifest does not match the pinned fixture lock/platform")
        dns_image = dns_assets["imageId"]
        if not isinstance(dns_image, str) or not dns_image.startswith("sha256:") or len(dns_image) != 71:
            raise RuntimeError("DNS OS image must be an immutable local SHA256 image ID")
        dns_description = json.loads(run("docker", "image", "inspect", dns_image, capture=True))[0]
        if (dns_description["Id"] != dns_image or dns_description["Os"] != "linux"
                or dns_description["Architecture"] != platform.split("/")[1]
                or dns_description["Config"].get("Volumes")):
            raise RuntimeError("DNS OS image identity/platform/volumes mismatch")
        shutil.copyfile(ROOT / "dev/dns-fixture/namespace.sh", report / "dns-support/namespace.sh")
    traversal_image = None
    traversal_assets = None
    if args.traversal == "nat":
        traversal_assets = json.loads(args.traversal_image_manifest.read_text())
        lock_path = ROOT / "dev/traversal-fixture/assets.lock.json"
        lock = json.loads(lock_path.read_text())
        if (traversal_assets.get("status") != "built" or traversal_assets.get("platform") != platform
                or traversal_assets.get("base") != lock["base"] or traversal_assets.get("lockSha256") != sha256(lock_path)
                or not set(lock["packages"]).issubset({item["name"] for item in traversal_assets.get("packages", [])})):
            raise RuntimeError("Traversal OS manifest does not match the locked router assets")
        traversal_image = traversal_assets["imageId"]
        if not isinstance(traversal_image, str) or not traversal_image.startswith("sha256:") or len(traversal_image) != 71:
            raise RuntimeError("Traversal OS image must be an immutable local SHA256 ID")
        inspected_image = json.loads(run("docker", "image", "inspect", traversal_image, capture=True))[0]
        if (inspected_image["Id"] != traversal_image or inspected_image["Os"] != "linux"
                or inspected_image["Architecture"] != platform.split("/")[1] or inspected_image["Config"].get("Volumes")):
            raise RuntimeError("Traversal OS image identity/platform/volumes mismatch")
        shutil.copyfile(ROOT / "dev/traversal-fixture/router.sh", report / "traversal-support/router.sh")
    if fingerprint() != source:
        raise RuntimeError("Sources changed before runtime start.")
    label_key = "io.meshway.runtime-fixture"
    label = label_key + "=" + identity
    containers = {}
    network = None
    networks = {}
    actor_networks = {}
    routers = {}
    manifest = {**artifacts, "image": image, "imageId": description["Id"], "runtimeOsAssets": runtime_os, "run": identity,
                 "loadBeforeRuntime": load,
                "dirty": bool(run("git", "status", "--porcelain", capture=True).strip()),
                 "passed": False, "processes": {}, "resources": {"containers": containers},
                  "carrier": args.transport,
                  "dnsBackend": args.dns_backend,
                   "dnsOsAssets": dns_assets,
                   "traversal": args.traversal,
                   "reportedEndpoints": args.reported_endpoints,
                   "traversalOsAssets": traversal_assets,
                   "traversalScope": "outer IPv4, two port-preserving Linux SNATs; optional same-edge TCP fallback; no mapping-rotation claim" if args.traversal == "nat" else None,
                   "traversalFallback": {"status": "notRun"} if args.traversal_fallback else None,
                  "scope": "diagnostic-short" if args.short else "acceptance",
                  "trafficSeconds": 30 if args.short else 125,
                "tests": {name: {"status": "notRun", "test": test} for name, test in KERNEL_TESTS.items()},
                  "bootstrapHttpsTest": {"status": "notRun"},
                   "platformTests": {},
                   "daemonCleanupTest": {"status": "notRun", "test": LIFECYCLE_TEST},
                "cleanup": {}}
    metadata = report / "report.json"
    write_json(metadata, manifest)

    def create(role, arguments, address=None, address6=None, private=False, tun=False, admin=False,
               prepare=False, executable="meshway-runtime-fixture", environment=None):
        dns_actor = args.dns_backend != "off" and role in ("a", "r", "b")
        attachment = actor_networks.get(role, network) if address else "none"
        if role == "underlay-negative":
            # This test deliberately captures its *underlay peer* with a sink.
            # Host-inherited loose RPF otherwise drops replies using that sink,
            # before the marked-socket contract can be observed. Scope the
            # baseline to this one disposable namespace, never a host sysctl.
            attachment = (f"name={attachment},driver-opt=com.docker.network.endpoint.sysctls="
                          "net.ipv4.conf.IFNAME.rp_filter=0")
        command = ["docker", "create", "--pull", "never", "--name", f"meshway-runtime-{identity.lower()}-{role}",
                   "--entrypoint", "",
                   "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                   "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "768m",
                   "--pids-limit", "128", "--tmpfs", "/state:rw,nosuid,nodev,mode=700",
                   "--env", "TMPDIR=/state",
                   "--network", attachment]
        command.extend(bind_access_options())
        if role == "underlay-negative":
            command.extend(["--sysctl", "net.ipv4.conf.all.rp_filter=0",
                            "--sysctl", "net.ipv4.conf.default.rp_filter=0"])
        if address:
            command.extend(["--ip", str(address)])
        if address6:
            command.extend(["--ip6", str(address6)])
        if role in ("underlay-dns-anchor", "underlay-dns-foreign", "daemon-cleanup"):
            command.extend(["--tmpfs", "/run:rw,nosuid,nodev,mode=755"])
        if role in PLATFORM_BINARIES:
            command.extend(["--workdir", "/state"])
        for key, value in (environment or {}).items():
            command.extend(["--env", f"{key}={value}"])
        if args.traversal == "nat" and role in ("a", "r", "b"):
            command.extend(["--env", "MESHWAY_TRAVERSAL_OWNED=1"])
        if dns_actor:
            command.extend(mount_namespace_options())
            command.extend(["--tmpfs", "/run:rw,nosuid,nodev,mode=755",
                            "--env", "MESHWAY_DNS_FIXTURE_OWNED=1",
                            "--env", "MESHWAY_DNS_BACKEND=" + args.dns_backend,
                            "--env", "MESHWAY_DNS_UPSTREAM=" + addresses["controller"],
                            "--env", "DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket"])
            capabilities = ["SYS_ADMIN", "CHOWN", "NET_BIND_SERVICE"]
            if args.dns_backend == "resolved":
                # resolved's documented privilege drop/bind path and the
                # production owner's /proc PID-netns verification across UID.
                capabilities += ["NET_RAW", "SETUID", "SETGID", "SETPCAP", "DAC_OVERRIDE", "SYS_PTRACE", "FOWNER", "KILL"]
            for capability in capabilities:
                command.extend(["--cap-add", capability])
        if args.dns_backend != "off" and role == "controller":
            command.extend(["--cap-add", "NET_BIND_SERVICE"])
        mounts = [("bin", "/fixtures", True), ("public", "/public", not prepare),
                  ("signals", "/signals", False)]
        if private:
            mounts.append(("controller", "/controller", not prepare))
        if dns_actor:
            mounts.append(("dns-support", "/dns-fixture", True))
        if not prepare:
            mounts.append((role, "/evidence", False))
        if role in ("underlay-restart-a", "underlay-restart-b"):
            mounts.append(("restart", "/restart", False))
        for source_name, destination, readonly in mounts:
            command.extend(["--mount", f"type=bind,source={report / source_name},target={destination}" + (",readonly" if readonly else "")])
        if tun or admin:
            command.extend(["--cap-add", "NET_ADMIN"])
        if tun:
            command.extend(["--device", "/dev/net/tun:/dev/net/tun"])
        if address6 or tun:
            command.extend(["--sysctl", "net.ipv6.conf.all.disable_ipv6=0", "--sysctl", "net.ipv6.conf.default.disable_ipv6=0"])
        command.append(dns_image if dns_actor else image)
        if dns_actor:
            command.extend(["/bin/sh", "/dns-fixture/namespace.sh"])
        command.append("/fixtures/" + executable)
        if executable == "meshway-runtime-fixture":
            if args.short:
                command.append("--short")
            command.extend(["--log-format", "json", "--transport", args.transport, "--dns-backend", args.dns_backend,
                            "--traversal", args.traversal])
            if args.traversal_fallback:
                command.append("--traversal-fallback")
            if args.reported_endpoints:
                command.append("--reported-endpoints")
        command.extend(arguments)
        identifier = run(*command, capture=True).strip()
        containers[role] = identifier
        manifest["resources"].setdefault("commands", {})[role] = command
        write_json(metadata, manifest)

    def execute(role):
        identifier = containers[role]
        with (report / f"{role}.log").open("w") as log:
            result = subprocess.run(["docker", "start", "-a", identifier], cwd=ROOT, text=True,
                                    stdout=log, stderr=subprocess.STDOUT, timeout=930, close_fds=True)
        state = json.loads(run("docker", "inspect", identifier, capture=True))[0]["State"]
        return {"attachExit": result.returncode, "exitCode": state["ExitCode"], "oomKilled": state["OOMKilled"], "running": state["Running"]}

    def owned_container(identifier):
        inspected = json.loads(run("docker", "inspect", identifier, capture=True))[0]
        if inspected["Config"]["Labels"].get(label_key) != identity:
            raise RuntimeError("container ownership label changed")
        return inspected

    def prepare_restart():
        # Unlike each actor's private /state tmpfs, this directory must survive
        # two containers. Give the empty, run-owned bind the same root ownership;
        # final export_evidence returns it to the host after both writers join.
        directory = report / "restart"
        if (report.is_symlink() or report.stat().st_uid != os.getuid()
                or directory.is_symlink() or directory.stat().st_uid != os.getuid()
                or any(directory.iterdir())):
            raise RuntimeError("restart preparation requires this run's empty owned directory")
        role = "restart-prepare"
        command = ["docker", "create", "--pull", "never", "--platform", platform,
                   "--name", f"meshway-runtime-{identity.lower()}-{role}", "--label", label,
                   "--network", "none", "--read-only", "--cap-drop", "ALL",
                   "--cap-add", "CHOWN", "--cap-add", "DAC_OVERRIDE",
                   "--security-opt", "no-new-privileges", "--cpus", "1", "--memory", "64m",
                   "--pids-limit", "32", "--entrypoint", "/bin/sh",
                   "--mount", f"type=bind,source={directory},target=/restart", image,
                   "-ec", "chown 0:0 /restart && chmod 700 /restart"]
        identifier = run(*command, capture=True).strip()
        containers[role] = identifier
        manifest["resources"].setdefault("commands", {})[role] = command
        write_json(metadata, manifest)
        try:
            result = execute(role)
            manifest["restartPreparation"] = result
            if any(result.values()):
                raise RuntimeError("restart private directory preparation failed")
        finally:
            owned_container(identifier)
            run("docker", "rm", "-f", identifier, capture=True)
            containers.pop(role)
            manifest["cleanup"][role] = True
            write_json(metadata, manifest)

    def start_router(actor):
        role = "router-" + actor
        router = routers[actor]
        command = ["docker", "create", "--pull", "never", "--name", f"meshway-runtime-{identity.lower()}-{role}",
                   "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                   "--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW", "--cap-add", "SETUID", "--cap-add", "SETGID",
                   "--security-opt", "no-new-privileges", "--cpus", "1", "--memory", "256m", "--pids-limit", "32",
                   "--tmpfs", "/run:rw,nosuid,nodev,mode=700", "--env", "MESHWAY_TRAVERSAL_OWNED=1",
                   "--network", network, "--ip", router["wan"], "--ip6", router["wan6"],
                   "--sysctl", "net.ipv4.ip_forward=1", "--sysctl", "net.ipv6.conf.all.forwarding=0",
                    "--sysctl", "net.ipv6.conf.all.disable_ipv6=0", "--sysctl", "net.ipv6.conf.default.disable_ipv6=0"]
        command.extend(bind_access_options())
        for source_name, destination, readonly in ((role, "/evidence", False), ("signals", "/signals", False),
                                                   ("traversal-support", "/traversal-fixture", True)):
            command.extend(["--mount", f"type=bind,source={report / source_name},target={destination}" + (",readonly" if readonly else "")])
        if args.traversal_fallback:
            command.extend(["--env", "MESHWAY_TRAVERSAL_FALLBACK=1", "--env",
                            "MESHWAY_TRAVERSAL_PEER_WAN=" + routers["r" if actor == "a" else "a"]["wan"]])
        command.extend([traversal_image, "/bin/sh", "/traversal-fixture/router.sh", role,
                        router["private"], router["wan"], addresses[actor]])
        identifier = run(*command, capture=True).strip()
        containers[role] = identifier
        manifest["resources"].setdefault("commands", {})[role] = command
        write_json(metadata, manifest)
        owned_container(identifier)
        connect = ["docker", "network", "connect", "--ip", router["private"], "--ip6", router["private6"],
                   actor_networks[actor], identifier]
        run(*connect, capture=True)
        manifest["resources"].setdefault("networkConnect", {})[role] = connect
        run("docker", "start", identifier, capture=True)
        deadline = time.monotonic() + 20
        ready = report / f"signals/{role}-ready.json"
        while not ready.exists():
            if time.monotonic() >= deadline or not owned_container(identifier)["State"]["Running"]:
                raise RuntimeError("NAT router did not become ready: " + role)
            time.sleep(0.1)
        export_evidence(report, image, platform, "runtime", identity, [ready])
        if json.loads(ready.read_text()) != {"kind": "ready"}:
            raise RuntimeError("invalid NAT router readiness")
        manifest["resources"].setdefault("routerInspections", {})[role] = owned_container(identifier)
        write_json(metadata, manifest)

    def stop_routers():
        write_json(report / "signals/routers-stop.json", {"kind": "ready"})
        failures = []
        for actor in routers:
            role = "router-" + actor
            if role not in containers or role in manifest.get("routerProcesses", {}):
                continue
            identifier = containers[role]
            try:
                owned_container(identifier)
                run("docker", "wait", identifier, capture=True, timeout=20)
                inspected = owned_container(identifier)
                state = inspected["State"]
                (report / f"{role}.log").write_text(run("docker", "logs", identifier, capture=True))
                manifest.setdefault("routerProcesses", {})[role] = {
                    "exitCode": state["ExitCode"], "running": state["Running"], "oomKilled": state["OOMKilled"]}
                if state["ExitCode"] or state["Running"] or state["OOMKilled"]:
                    raise RuntimeError("router capture did not stop/join cleanly: " + role)
            except Exception as error:
                failures.append(error)
        write_json(metadata, manifest)
        if failures:
            raise ExceptionGroup("NAT router shutdown failed", failures)

    failure = None
    try:
        prepare_restart()
        create("prepare", ["prepare"], private=True, prepare=True)
        preparation = execute("prepare")
        manifest["preparation"] = preparation
        if preparation["attachExit"] or preparation["exitCode"] or preparation["oomKilled"] or preparation["running"]:
            raise RuntimeError("runtime TLS preparation failed")
        create("daemon-cleanup", ["--ignored", "--exact", LIFECYCLE_TEST, "--nocapture", "--test-threads=1"],
               executable=LIFECYCLE_BINARY, admin=True,
               environment={"MESHWAY_DNS_ISOLATED": "1", "MESHWAY_UNDERLAY_ISOLATED": "1",
                            "MESHWAY_TEST_DAEMON_BINARY": "/fixtures/meshwayd"})
        result = execute("daemon-cleanup")
        passed = not any(result.values()) and "test result: ok." in (report / "daemon-cleanup.log").read_text()
        manifest["daemonCleanupTest"] = {"passed": passed, "test": LIFECYCLE_TEST, **result}
        write_json(metadata, manifest)
        if not passed:
            raise RuntimeError("Linux daemon blocked-down cleanup retry failed")
        for role, (_, binary) in PLATFORM_BINARIES.items():
            create(role, ["--include-ignored", "--skip", OWNER_CONFLICT_TEST, "--nocapture", "--test-threads=4"], executable=binary,
                   environment={"MESHWAY_DNS_ISOLATED": "1", "MESHWAY_DNS_BIND_MOUNT_FILE": "/etc/resolv.conf"})
            result = execute(role)
            passed = not any(result.values()) and "test result: ok." in (report / f"{role}.log").read_text()
            manifest["platformTests"][role] = {"passed": passed, **result}
            write_json(metadata, manifest)
            if not passed:
                raise RuntimeError("Linux DNS/daemon platform tests failed: " + role)
        create("daemon-owner-conflict", ["--ignored", "--exact", OWNER_CONFLICT_TEST, "--nocapture", "--test-threads=1"],
               executable=PLATFORM_BINARIES["platform-daemon"][1], tun=True,
               environment={"MESHWAY_UNDERLAY_ISOLATED": "1"})
        result = execute("daemon-owner-conflict")
        passed = not any(result.values()) and "test result: ok. 1 passed; 0 failed; 0 ignored;" in (report / "daemon-owner-conflict.log").read_text()
        manifest["ownerConflictTest"] = {"passed": passed, "test": OWNER_CONFLICT_TEST, **result}
        write_json(metadata, manifest)
        if not passed:
            raise RuntimeError("live TUN owner conflict retry failed")
        random = uuid.uuid4().hex
        ipv6_requested = ipaddress.ip_network(f"2001:db8:{random[:4]}:{random[4:8]}::/64")
        network = run("docker", "network", "create", "--internal", "--ipv6", "--subnet", str(ipv6_requested), "--label", label,
                      "meshway-runtime-" + identity.lower(), capture=True).strip()
        networks["wan"] = network
        manifest["resources"]["networks"] = networks
        manifest["resources"]["network"] = network
        write_json(metadata, manifest)
        inspected = json.loads(run("docker", "network", "inspect", network, capture=True))[0]
        if inspected["Labels"].get(label_key) != identity or not inspected["Internal"] or inspected["Driver"] != "bridge":
            raise RuntimeError("WAN is not the owned internal fixture bridge")
        manifest["resources"]["networkInspections"] = {"wan": inspected}
        configurations = inspected["IPAM"]["Config"]
        ipv4 = None
        ipv6 = None
        for config in configurations:
            subnet = ipaddress.ip_network(config["Subnet"])
            managed = ipaddress.ip_network("100.64.0.0/10" if subnet.version == 4 else "fc00::/7")
            if subnet.overlaps(managed):
                raise RuntimeError("isolated Docker underlay overlaps managed mesh pools")
            if subnet.version == 4:
                ipv4 = subnet
            else:
                ipv6 = subnet
        if ipv4 is None or ipv4.num_addresses < 32:
            raise RuntimeError("fixture needs an isolated IPv4 subnet with at least 32 addresses")
        if ipv6 != ipv6_requested:
            raise RuntimeError("fixture IPv6 underlay differs from requested documentation-only prefix")
        network_roles = (*ROLES, "underlay-peer", "bootstrap", *KERNEL_TESTS)
        addresses = {role: str(ipv4.network_address + 10 + index) for index, role in enumerate(network_roles)}
        addresses6 = {role: str(ipv6.network_address + 10 + index) for index, role in enumerate(network_roles)}
        gateways = {config.get("Gateway") for config in configurations}
        if gateways.intersection((*addresses.values(), *addresses6.values())):
            raise RuntimeError("fixture address conflicts with isolated gateway")
        if args.traversal == "nat":
            subnets = [ipv4, ipv6]
            for index, role in enumerate(("a", "r")):
                salt = uuid.uuid4().hex
                requested6 = ipaddress.ip_network(f"2001:db8:{salt[:4]}:{salt[4:8]}::/64")
                identifier = run("docker", "network", "create", "--internal", "--ipv6", "--subnet", str(requested6),
                                 "--label", label, f"meshway-runtime-{identity.lower()}-private-{role}", capture=True).strip()
                networks["private-" + role] = identifier
                actor_networks[role] = identifier
                write_json(metadata, manifest)
                inspected_private = json.loads(run("docker", "network", "inspect", identifier, capture=True))[0]
                if (inspected_private["Labels"].get(label_key) != identity
                        or not inspected_private["Internal"] or inspected_private["Driver"] != "bridge"):
                    raise RuntimeError("NAT LAN is not the owned internal fixture bridge")
                manifest["resources"]["networkInspections"]["private-" + role] = inspected_private
                configurations_private = inspected_private["IPAM"]["Config"]
                private4 = None
                private6 = None
                for config in configurations_private:
                    subnet = ipaddress.ip_network(config["Subnet"])
                    managed = ipaddress.ip_network("100.64.0.0/10" if subnet.version == 4 else "fc00::/7")
                    if subnet.overlaps(managed) or any(subnet.version == previous.version and subnet.overlaps(previous) for previous in subnets):
                        raise RuntimeError("NAT private bridge overlaps another fixture or Mesh subnet")
                    subnets.append(subnet)
                    if subnet.version == 4:
                        private4 = subnet
                    else:
                        private6 = subnet
                if private4 is None or private4.num_addresses < 32 or private6 != requested6:
                    raise RuntimeError("Unexpected private NAT bridge address allocation")
                addresses[role] = str(private4.network_address + 10)
                addresses6[role] = str(private6.network_address + 10)
                routers[role] = {"private": str(private4.network_address + 2), "private6": str(private6.network_address + 2),
                                 "wan": str(ipv4.network_address + 28 + index), "wan6": str(ipv6.network_address + 28 + index)}
                allocated = {addresses[role], addresses6[role], *routers[role].values()}
                if allocated.intersection({config.get("Gateway") for config in configurations_private} | gateways):
                    raise RuntimeError("NAT fixture address conflicts with Docker gateway")
            manifest["natRouters"] = routers
        write_json(report / "public/underlay.json", {"controller": addresses["controller"],
                    "actors": {role: addresses[role] for role in ("a", "r", "b")},
                    "routers": {role: {"private": router["private"], "wan": router["wan"]} for role, router in routers.items()}})
        manifest["underlay"] = addresses
        manifest["underlay6"] = addresses6
        create("underlay-peer", ["underlay-peer", "--ipv4", addresses["underlay-peer"], "--ipv6", addresses6["underlay-peer"]],
               address=addresses["underlay-peer"], address6=addresses6["underlay-peer"])
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            peer = executor.submit(execute, "underlay-peer")
            restart = None
            def kernel_result(role, result):
                passed = (not result["attachExit"] and not result["exitCode"] and not result["oomKilled"] and not result["running"]
                          and "test result: ok. 1 passed; 0 failed; 0 ignored;" in (report / f"{role}.log").read_text())
                manifest["tests"][role] = {"test": KERNEL_TESTS[role], "passed": passed, **result}
                write_json(metadata, manifest)
                if not passed:
                    raise RuntimeError("production underlay kernel conformance failed")
            try:
                deadline = time.monotonic() + 30
                ready = report / "signals/underlay-peer-ready.json"
                while not ready.exists():
                    if peer.done():
                        raise RuntimeError("remote underlay peer exited before readiness")
                    if time.monotonic() >= deadline:
                        raise TimeoutError("remote underlay peer readiness timed out")
                    time.sleep(0.1)
                export_evidence(report, image, platform, "runtime", identity, [ready])
                if json.loads(ready.read_text()) != {"kind": "ready"}:
                    raise RuntimeError("invalid remote underlay peer readiness")
                for role, test in KERNEL_TESTS.items():
                    environment = {"MESHWAY_UNDERLAY_ISOLATED": "1", "MESHWAY_DNS_ISOLATED": "1"}
                    if role in ("underlay-restart-a", "underlay-restart-b"):
                        environment["MESHWAY_RESTART_PHASE"] = role[-1]
                    if role == "underlay-negative":
                        environment.update(MESHWAY_UNDERLAY_PEER4=addresses["underlay-peer"] + ":42427",
                                           MESHWAY_UNDERLAY_PEER6="[" + addresses6["underlay-peer"] + "]:42427")
                    create(role, ["--include-ignored", "--exact", test, "--nocapture", "--test-threads=1"],
                           address=addresses[role], address6=addresses6[role], admin=not role.startswith("underlay-journal-"),
                           tun=role == "underlay-local-conflict",
                           executable=KERNEL_BINARY, environment=environment)
                    if role == "underlay-restart-a":
                        restart = executor.submit(execute, role)
                        deadline = time.monotonic() + 30
                        while not (report / "signals/underlay-restart-ready.json").exists():
                            if restart.done() or time.monotonic() >= deadline:
                                raise RuntimeError("restart namespace pin did not become ready")
                            time.sleep(0.1)
                        continue
                    try:
                        result = execute(role)
                    finally:
                        if role == "underlay-restart-b":
                            write_json(report / "signals/underlay-restart-stop.json", {"kind": "ready"})
                            kernel_result("underlay-restart-a", restart.result())
                    kernel_result(role, result)
            finally:
                write_json(report / "signals/underlay-restart-stop.json", {"kind": "ready"})
                write_json(report / "signals/underlay-peer-stop.json", {"kind": "ready"})
                manifest["underlayPeer"] = peer.result()
                if restart is not None:
                    restart.result()
                write_json(metadata, manifest)
        if any(manifest["underlayPeer"].values()):
            raise RuntimeError("remote underlay peer did not stop cleanly")
        for role in routers:
            start_router(role)
        create("controller", ["controller"], address=addresses["controller"], address6=addresses6["controller"], private=True)
        for role in ("a", "r", "b"):
            create(role, ["actor", "--role", role], address=addresses[role], address6=addresses6[role], tun=True)
        create("bootstrap", ["--ignored", "--exact", "fixed_origin_https_survives_explicit_network_down", "--nocapture"],
               address=addresses["bootstrap"], address6=addresses6["bootstrap"], admin=True,
               executable=BOOTSTRAP_BINARY, environment={"MESHWAY_UNDERLAY_ISOLATED": "1",
                    "MESHWAY_UNDERLAY_ORIGIN": "https://meshway-packet-controller:8443/",
                    "MESHWAY_UNDERLAY_ORIGIN_IP": addresses["controller"],
                    "MESHWAY_UNDERLAY_CA": "/public/server.pem"})
        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as executor:
            futures = {executor.submit(execute, role): role for role in ROLES}
            def interface_change():
                replacement = str(ipv4.network_address + 30)
                if replacement in addresses.values() or replacement in gateways:
                    raise RuntimeError("interface replacement address is not reserved")
                write_json(report / "signals/interface-addresses.json", {"old": addresses["r"], "new": replacement})
                def link_addresses():
                    links = json.loads(run("docker", "exec", containers["r"], "ip", "-json", "address", "show", "dev", "eth0", capture=True))
                    return [address for link in links for address in link["addr_info"]
                            if address["family"] == "inet6" and address["scope"] == "link"]
                original_link_addresses = None
                for stage, address in (("changed", replacement), ("restored", addresses["r"])):
                    deadline = time.monotonic() + 900
                    while not (report / f"signals/interface-request-{stage}.json").exists():
                        if any(future.done() for future in futures) or time.monotonic() >= deadline:
                            raise RuntimeError("interface replacement rendezvous failed")
                        time.sleep(0.1)
                    owned_container(containers["r"])
                    if stage == "changed":
                        original_link_addresses = link_addresses()
                        if len(original_link_addresses) != 1:
                            raise RuntimeError("expected this fixture's settled IPv6 link-local address")
                    run("docker", "network", "disconnect", network, containers["r"], capture=True)
                    run("docker", "network", "connect", "--ip", address, "--ip6", addresses6["r"], network, containers["r"], capture=True)
                    if stage == "restored":
                        # Docker assigns a new MAC to the replacement veth. Restore
                        # only our fixture's original link-local address too, so
                        # the existing exact-down baseline still compares the same
                        # underlay; never remove or normalize Meshway resources.
                        original = original_link_addresses[0]
                        current = link_addresses()
                        run("docker", "exec", containers["r"], "ip", "-6", "addr", "replace",
                            f"{original['local']}/{original['prefixlen']}", "dev", "eth0", "scope", "link", "nodad", capture=True)
                        for item in current:
                            if item["local"] != original["local"]:
                                run("docker", "exec", containers["r"], "ip", "-6", "addr", "del",
                                    f"{item['local']}/{item['prefixlen']}", "dev", "eth0", capture=True)
                    (report / f"interface-{stage}-addresses.json").write_text(run("docker", "exec", containers["r"], "ip", "-json", "address", capture=True))
                    write_json(report / f"signals/interface-{stage}.json", {"kind": "ready"})
            interface = executor.submit(interface_change) if args.reported_endpoints else None
            def bootstrap_check():
                deadline = time.monotonic() + 30
                while not (report / "signals/enroll.json").exists():
                    if time.monotonic() >= deadline:
                        raise TimeoutError("HTTPS Server bootstrap readiness timed out")
                    time.sleep(0.1)
                return execute("bootstrap")
            bootstrap = executor.submit(bootstrap_check)
            for future in concurrent.futures.as_completed(futures):
                role = futures[future]
                manifest["processes"][role] = future.result()
                write_json(metadata, manifest)
            result = bootstrap.result()
            if interface:
                interface.result()
            passed = (not any(result.values()) and "test result: ok. 1 passed; 0 failed; 0 ignored;" in (report / "bootstrap.log").read_text())
            manifest["bootstrapHttpsTest"] = {"passed": passed, **result}
            if not passed:
                raise RuntimeError("HTTPS bootstrap did not survive explicit network down")
        if fingerprint() != source:
            raise RuntimeError("sources changed while validating candidate binaries")
        if routers:
            stop_routers()
        export_evidence(report, image, platform, "runtime", identity)
        if any(item["attachExit"] or item["exitCode"] or item["oomKilled"] or item["running"] for item in manifest["processes"].values()):
            raise RuntimeError("real daemon/CLI conformance failed; inspect private actor evidence")
        for role in ("a", "r", "b"):
            actor = json.loads((report / f"controller/report-{role}.json").read_text())
            manifest.setdefault("actors", {})[role] = actor
            if args.traversal == "nat":
                inspected_actor = owned_container(containers[role])
                attached = inspected_actor["NetworkSettings"]["Networks"].values()
                if {item["NetworkID"] for item in attached} != {actor_networks.get(role, network)}:
                    raise RuntimeError("Actor has an unexpected network attachment/bypass")
                manifest["resources"].setdefault("actorInspections", {})[role] = inspected_actor
        proof_names = ["ping-shared-user.json", "ping-permission.json"]
        if args.traversal == "nat":
            proof_names.extend(["ping-nat-channel-proofs.json", "traversal-channel-continuity.json"])
        manifest["pingProofs"] = {}
        for name in proof_names:
            path = report / "controller" / name
            if not path.is_file() or path.stat().st_size > 1024 * 1024:
                raise RuntimeError("missing or oversized validated ping evidence: " + name)
            manifest["pingProofs"][name] = {"sha256": sha256(path), "data": json.loads(path.read_text())}
        if args.traversal == "nat":
            stop_routers()
            spec = importlib.util.spec_from_file_location("meshway_traversal_evidence", ROOT / "dev/traversal-fixture/evidence.py")
            if spec is None or spec.loader is None:
                raise RuntimeError("Traversal evidence reader unavailable")
            evidence = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(evidence)
            manifest["traversalPackets"] = evidence.verify(report, addresses, routers, fallback=args.traversal_fallback, short=args.short)
            if args.traversal_fallback:
                manifest["traversalFallback"] = {"status": "passed", **manifest["traversalPackets"]["fallback"]}
        manifest["passed"] = True
    except BaseException as error:
        failure = error
        manifest["passed"] = False
        manifest["failure"] = type(error).__name__
        if args.traversal_fallback and (report / "signals/enable-fallback.json").exists():
            manifest["traversalFallback"] = {"status": "failed"}
    finally:
        cleanup_errors = []
        if routers:
            try:
                stop_routers()
            except Exception as error:
                cleanup_errors.append(error)
        for role, identifier in containers.items():
            try:
                labels = json.loads(run("docker", "inspect", identifier, capture=True))[0]["Config"]["Labels"]
                if labels.get(label_key) != identity:
                    raise RuntimeError("container ownership label changed")
                run("docker", "rm", "-f", identifier, capture=True)
                manifest["cleanup"][role] = True
            except Exception as error:
                cleanup_errors.append(error)
                manifest["cleanup"][role] = False
        for name, identifier in reversed(list(networks.items())):
            try:
                labels = json.loads(run("docker", "network", "inspect", identifier, capture=True))[0]["Labels"]
                if labels.get(label_key) != identity:
                    raise RuntimeError("network ownership label changed")
                run("docker", "network", "rm", identifier, capture=True)
                manifest["cleanup"]["network" if name == "wan" else "network-" + name] = True
            except Exception as error:
                cleanup_errors.append(error)
                manifest["cleanup"]["network" if name == "wan" else "network-" + name] = False
        try:
            export_evidence(report, image, platform, "runtime", identity)
        except Exception as error:
            cleanup_errors.append(error)
        if cleanup_errors:
            manifest["passed"] = False
        manifest["runtimeElapsedSeconds"] = time.monotonic() - runtime_started
        manifest["sourceDigestAfter"] = fingerprint()
        if manifest["sourceDigestAfter"] != source:
            manifest["passed"] = False
        write_json(metadata, manifest)
        print(report)
        if cleanup_errors:
            raise ExceptionGroup("runtime fixture cleanup failed", cleanup_errors) from failure
    if failure:
        raise failure
    print("Real daemon/CLI runtime conformance passed; private evidence:", metadata)


if __name__ == "__main__":
    main()
