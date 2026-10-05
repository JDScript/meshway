#!/usr/bin/env python3
"""Real meshwayd/CLI Linux conformance; host-only build, disposable namespaces.

Builds once on the host, then runs precompiled actors in isolated containers.
"""
import argparse
import concurrent.futures
import datetime
import fcntl
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid

from node_lab import ROOT, check_elf, docker_target, run, sha256
from verify_tun import pinned_image

BINARIES = ("meshwayd", "meshway", "meshway-runtime-fixture")
KERNEL_BINARY = "meshway-underlay-kernel-tests"
BOOTSTRAP_BINARY = "meshway-underlay-bootstrap-tests"
PLATFORM_BINARIES = {"platform-dns": ("meshway_dns", "meshway-dns-platform-tests"),
                     "platform-daemon": ("meshway_daemon", "meshway-daemon-platform-tests")}
ARTIFACTS = (*BINARIES, KERNEL_BINARY, BOOTSTRAP_BINARY, *(binary for _, binary in PLATFORM_BINARIES.values()))
ROLES = ("controller", "a", "r", "b")
KERNEL_TESTS = {
    "underlay-negative": "linux::kernel::tests::negative_capture_marked_bypass_recovery_and_exact_down",
    "underlay-foreign": "linux::kernel::tests::foreign_same_priority_is_retained_and_blocks_cleanup",
    "underlay-datagram": "linux::datagram::tests::actual_udp_mark_truncation_and_unpolled_owner_release",
    "underlay-dns-anchor": "linux::kernel::tests::dns_anchor_survives_suspend_and_owner_restart_until_explicit_removal",
    "underlay-dns-foreign": "linux::kernel::tests::foreign_dns_anchor_mutation_blocks_cleanup_without_removing_the_interface",
}


def fingerprint():
    paths = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "mise.toml", "mise.lock")]
    for directory in ("crates", "apps/meshway-server", "apps/meshway-daemon", "apps/meshway-cli",
                      "tools/meshway-packet-fixture", "tools/meshway-runtime-fixture", "dev/dns-fixture"):
        # Include SQL and other compile-time include_str/include_bytes inputs,
        # not just Rust modules and manifests.
        paths.extend(path for path in (ROOT / directory).rglob("*")
                     if path.is_file() and not any(part in {"target", "node_modules", "__pycache__", ".git"}
                                                  for part in path.relative_to(ROOT / directory).parts)
                     and path.name != ".DS_Store")
    paths.extend(ROOT / "scripts" / name for name in
                 ("verify_runtime.py", "node_lab.py", "verify_tun.py"))
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


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
        shutil.copyfile(binary, directory / name)
        (directory / name).chmod(0o500)
    write_json(directory / "source-manifest.json", manifest)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-only", action="store_true", help="host build and source manifest only")
    parser.add_argument("--artifacts", type=Path, help="reuse a --build-only bin directory with matching source manifest")
    parser.add_argument("--transport", choices=("native-tcp", "native-udp"), default="native-tcp",
                        help="Server-configured outer carrier; never a daemon-local policy override")
    parser.add_argument("--dns-backend", choices=("off", "resolv-conf", "resolved"), default="off",
                        help="real libc DNS conformance in an owned private mount namespace")
    parser.add_argument("--dns-image-manifest", type=Path,
                        help="parent-built dev/dns-fixture/build.py immutable OS image manifest")
    args = parser.parse_args()
    if not args.build_only and (args.dns_backend != "off") != bool(args.dns_image_manifest):
        parser.error("DNS scenarios require --dns-image-manifest; IP-only runs must omit it")
    os.umask(0o077)
    target, platform, machine = docker_target()  # read-only engine architecture query
    common = Path(run("git", "rev-parse", "--git-common-dir", capture=True).strip())
    repository = (ROOT / common).resolve().parent
    root = repository / ".meshway/validation/runtime"
    root.mkdir(parents=True, exist_ok=True)
    identity = datetime.datetime.now(datetime.UTC).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    report = root / identity
    for name in ("bin", "public", "public/empty-cert-dir", "controller", "signals", "dns-support", *ROLES,
                 "underlay-peer", "bootstrap", *KERNEL_TESTS, *PLATFORM_BINARIES):
        (report / name).mkdir(parents=True, exist_ok=True)
    source = fingerprint()
    # One source/cache owner per invocation; never leak the lock FD to Cargo,
    # Docker attach processes, or any later runtime child.
    with (root / "build.lock").open("a") as lock:
        os.set_inheritable(lock.fileno(), False)
        fcntl.flock(lock, fcntl.LOCK_EX)
        artifacts = (import_artifacts(args.artifacts.resolve(), report / "bin", target, platform, machine, source)
                     if args.artifacts else build(report / "bin", target, platform, machine, source))
    if args.build_only:
        print(report / "bin")
        return
    image = pinned_image(platform, report / "bin")
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
    if fingerprint() != source:
        raise RuntimeError("Sources changed before runtime start.")
    label_key = "io.meshway.runtime-fixture"
    label = label_key + "=" + identity
    containers = {}
    network = None
    manifest = {**artifacts, "image": image, "imageId": description["Id"], "run": identity,
                "dirty": bool(run("git", "status", "--porcelain", capture=True).strip()),
                 "passed": False, "processes": {}, "resources": {"containers": containers},
                  "carrier": args.transport,
                  "dnsBackend": args.dns_backend,
                  "dnsOsAssets": dns_assets,
                 "scope": "real meshwayd/CLI; Server browser approval; A-R-B; IPv4/IPv6 UDP/TCP TUN; demand renewal; Access revocation; lease loss; offline down",
                "tests": {name: {"status": "notRun", "test": test} for name, test in KERNEL_TESTS.items()},
                  "bootstrapHttpsTest": {"status": "notRun"},
                  "platformTests": {},
                "cleanup": {}}
    metadata = report / "report.json"
    write_json(metadata, manifest)

    def create(role, arguments, address=None, address6=None, private=False, tun=False, admin=False,
               prepare=False, executable="meshway-runtime-fixture", environment=None):
        dns_actor = args.dns_backend != "off" and role in ("a", "r", "b")
        command = ["docker", "create", "--pull", "never", "--name", f"meshway-runtime-{identity.lower()}-{role}",
                   "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                   "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "768m",
                   "--pids-limit", "128", "--tmpfs", "/state:rw,nosuid,nodev,mode=700",
                   "--env", "TMPDIR=/state",
                   "--network", network if address else "none"]
        if address:
            command.extend(["--ip", str(address)])
        if address6:
            command.extend(["--ip6", str(address6)])
        if role in ("underlay-dns-anchor", "underlay-dns-foreign"):
            command.extend(["--tmpfs", "/run:rw,nosuid,nodev,mode=755"])
        if role in PLATFORM_BINARIES:
            command.extend(["--workdir", "/state"])
        for key, value in (environment or {}).items():
            command.extend(["--env", f"{key}={value}"])
        if dns_actor:
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
            command.extend(["--log-format", "json", "--transport", args.transport, "--dns-backend", args.dns_backend])
        command.extend(arguments)
        identifier = run(*command, capture=True).strip()
        containers[role] = identifier
        manifest["resources"].setdefault("commands", {})[role] = command
        write_json(metadata, manifest)

    def execute(role):
        identifier = containers[role]
        with (report / f"{role}.log").open("w") as log:
            result = subprocess.run(["docker", "start", "-a", identifier], cwd=ROOT, text=True,
                                    stdout=log, stderr=subprocess.STDOUT, timeout=450, close_fds=True)
        state = json.loads(run("docker", "inspect", identifier, capture=True))[0]["State"]
        return {"attachExit": result.returncode, "exitCode": state["ExitCode"], "oomKilled": state["OOMKilled"], "running": state["Running"]}

    failure = None
    try:
        create("prepare", ["prepare"], private=True, prepare=True)
        preparation = execute("prepare")
        manifest["preparation"] = preparation
        if preparation["attachExit"] or preparation["exitCode"] or preparation["oomKilled"] or preparation["running"]:
            raise RuntimeError("runtime TLS preparation failed")
        for role, (_, binary) in PLATFORM_BINARIES.items():
            create(role, ["--include-ignored", "--nocapture", "--test-threads=4"], executable=binary,
                   environment={"MESHWAY_DNS_ISOLATED": "1", "MESHWAY_DNS_BIND_MOUNT_FILE": "/etc/resolv.conf"})
            result = execute(role)
            passed = not any(result.values()) and "test result: ok." in (report / f"{role}.log").read_text()
            manifest["platformTests"][role] = {"passed": passed, **result}
            write_json(metadata, manifest)
            if not passed:
                raise RuntimeError("Linux DNS/daemon platform tests failed: " + role)
        random = uuid.uuid4().hex
        ipv6_requested = ipaddress.ip_network(f"2001:db8:{random[:4]}:{random[4:8]}::/64")
        network = run("docker", "network", "create", "--internal", "--ipv6", "--subnet", str(ipv6_requested), "--label", label,
                      "meshway-runtime-" + identity.lower(), capture=True).strip()
        manifest["resources"]["network"] = network
        write_json(metadata, manifest)
        inspected = json.loads(run("docker", "network", "inspect", network, capture=True))[0]
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
        write_json(report / "public/underlay.json", {"controller": addresses["controller"],
                    "actors": {role: addresses[role] for role in ("a", "r", "b")}})
        manifest["underlay"] = addresses
        manifest["underlay6"] = addresses6
        create("underlay-peer", ["underlay-peer", "--ipv4", addresses["underlay-peer"], "--ipv6", addresses6["underlay-peer"]],
               address=addresses["underlay-peer"], address6=addresses6["underlay-peer"])
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            peer = executor.submit(execute, "underlay-peer")
            try:
                deadline = time.monotonic() + 30
                ready = report / "signals/underlay-peer-ready.json"
                while not ready.exists():
                    if peer.done():
                        raise RuntimeError("remote underlay peer exited before readiness")
                    if time.monotonic() >= deadline:
                        raise TimeoutError("remote underlay peer readiness timed out")
                    time.sleep(0.1)
                if json.loads(ready.read_text()) != {"kind": "ready"}:
                    raise RuntimeError("invalid remote underlay peer readiness")
                for role, test in KERNEL_TESTS.items():
                    environment = {"MESHWAY_UNDERLAY_ISOLATED": "1", "MESHWAY_DNS_ISOLATED": "1"}
                    if role == "underlay-negative":
                        environment.update(MESHWAY_UNDERLAY_PEER4=addresses["underlay-peer"] + ":42427",
                                           MESHWAY_UNDERLAY_PEER6="[" + addresses6["underlay-peer"] + "]:42427")
                    create(role, ["--ignored", "--exact", test, "--nocapture", "--test-threads=1"],
                           address=addresses[role], address6=addresses6[role], admin=True,
                           executable=KERNEL_BINARY, environment=environment)
                    result = execute(role)
                    passed = (not result["attachExit"] and not result["exitCode"] and not result["oomKilled"] and not result["running"]
                              and "test result: ok. 1 passed; 0 failed; 0 ignored;" in (report / f"{role}.log").read_text())
                    manifest["tests"][role] = {"test": test, "passed": passed, **result}
                    write_json(metadata, manifest)
                    if not passed:
                        raise RuntimeError("production underlay kernel conformance failed")
            finally:
                write_json(report / "signals/underlay-peer-stop.json", {"kind": "ready"})
                manifest["underlayPeer"] = peer.result()
                write_json(metadata, manifest)
        if any(manifest["underlayPeer"].values()):
            raise RuntimeError("remote underlay peer did not stop cleanly")
        create("controller", ["controller"], address=addresses["controller"], address6=addresses6["controller"], private=True)
        for role in ("a", "r", "b"):
            create(role, ["actor", "--role", role], address=addresses[role], address6=addresses6[role], tun=True)
        create("bootstrap", ["--ignored", "--exact", "fixed_origin_https_survives_explicit_network_down", "--nocapture"],
               address=addresses["bootstrap"], address6=addresses6["bootstrap"], admin=True,
               executable=BOOTSTRAP_BINARY, environment={"MESHWAY_UNDERLAY_ISOLATED": "1",
                    "MESHWAY_UNDERLAY_ORIGIN": "https://meshway-packet-controller:8443/",
                    "MESHWAY_UNDERLAY_ORIGIN_IP": addresses["controller"],
                    "MESHWAY_UNDERLAY_CA": "/public/server.pem"})
        with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
            futures = {executor.submit(execute, role): role for role in ROLES}
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
            passed = (not any(result.values()) and "test result: ok. 1 passed; 0 failed; 0 ignored;" in (report / "bootstrap.log").read_text())
            manifest["bootstrapHttpsTest"] = {"passed": passed, **result}
            if not passed:
                raise RuntimeError("HTTPS bootstrap did not survive explicit network down")
        if fingerprint() != source:
            raise RuntimeError("sources changed while validating candidate binaries")
        if any(item["attachExit"] or item["exitCode"] or item["oomKilled"] or item["running"] for item in manifest["processes"].values()):
            raise RuntimeError("real daemon/CLI conformance failed; inspect private actor evidence")
        for role in ("a", "r", "b"):
            actor = json.loads((report / f"controller/report-{role}.json").read_text())
            manifest.setdefault("actors", {})[role] = actor
        manifest["passed"] = True
    except BaseException as error:
        failure = error
        manifest["passed"] = False
        manifest["failure"] = type(error).__name__
    finally:
        cleanup_errors = []
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
        if network:
            try:
                labels = json.loads(run("docker", "network", "inspect", network, capture=True))[0]["Labels"]
                if labels.get(label_key) != identity:
                    raise RuntimeError("network ownership label changed")
                run("docker", "network", "rm", network, capture=True)
                manifest["cleanup"]["network"] = True
            except Exception as error:
                cleanup_errors.append(error)
                manifest["cleanup"]["network"] = False
        if cleanup_errors:
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
