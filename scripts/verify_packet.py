#!/usr/bin/env python3
"""Protected TUN conformance in four disposable, isolated Linux namespaces."""
import concurrent.futures
import argparse
import ipaddress
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from node_lab import ROOT, check_elf, docker_target, run, sha256
from verify_tun import pinned_image
from verify_runtime import wait_for_idle
from verification_sources import fingerprint as source_fingerprint
from validation_support import build_lock, fixture_options, report_directory, save_artifact, import_artifact, run_id, bind_access_options, export_evidence


def fingerprint():
    return source_fingerprint(ROOT)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    fixture_options(parser)
    parser.add_argument("--runtime-artifacts", type=Path)
    parser.add_argument("--runtime-image-manifest", type=Path)
    parser.add_argument("--short", action="store_true")
    args = parser.parse_args()
    os.umask(0o077)
    if shutil.disk_usage(ROOT).free < 50 * 1024 ** 3:
        raise SystemExit("Linux build needs at least 50 GiB of free disk space.")
    target, platform, machine = docker_target()
    report = report_directory("packet", args.report_dir)
    identity = report.name
    for directory in ("bin", "public", "controller", "signals"):
        (report / directory).mkdir(parents=True)
    before = fingerprint()
    env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "target/node-kernel-tests"), CARGO_INCREMENTAL="0")
    binary = report / "bin/meshway-packet-fixture"
    if args.artifacts:
        import_artifact(args.artifacts, binary, before, target, platform, machine)
    else:
        with build_lock():
            run("rustup", "target", "add", target)
            run("cargo", "zigbuild", "--target", target, "--locked", "--all-features",
                "-p", "meshway-packet-fixture", "--bin", "meshway-packet-fixture", env=env)
            if fingerprint() != before:
                raise SystemExit("Sources changed during the build; repeat before verification.")
            shutil.copyfile(Path(env["CARGO_TARGET_DIR"]) / target / "debug/meshway-packet-fixture", binary)
            binary.chmod(0o500)
    check_elf(binary, machine)
    save_artifact(report / "bin", binary, before, target, platform)
    if args.build_only:
        print(report / "bin")
        return
    image = pinned_image(platform, report / "bin")
    load = wait_for_idle()
    label = "io.meshway.packet-fixture=" + identity
    containers = {}
    network = None
    manifest = {"revision": run("git", "rev-parse", "HEAD", capture=True).strip(),
                "dirty": bool(run("git", "status", "--porcelain", capture=True).strip()),
                "sourceDigest": before, "target": target, "platform": platform,
                "image": image, "sha256": sha256(binary), "passed": False,
                "scope": "diagnostic-short" if args.short else "acceptance",
                "loadBeforeRuntime": load,
                "coverage": "real HTTPS control, full-MTU IPv4/IPv6 UDP and MTU+1 rejection over TUN/Noise/relay; real daemon multi-MSS TCP and full-MTU regression"}
    metadata = report / "report.json"
    metadata.write_text(json.dumps(manifest, indent=2) + "\n")

    def create(role, arguments, connected=False, private=False, tun=False, writable_public=False):
        command = ["docker", "create", "--name", f"meshway-packet-{identity.lower()}-{role}",
                   "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                   "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "768m",
                    "--pids-limit", "128", "--tmpfs", "/state:rw,nosuid,nodev,mode=700"]
        command.extend(bind_access_options())
        if connected:
            command.extend(["--network", network, "--network-alias", "meshway-packet-" + role])
        else:
            command.extend(["--network", "none"])
        for source, destination, readonly in [
            ("bin", "/fixtures", True), ("public", "/public", not writable_public),
            ("signals", "/signals", False),
        ]:
            command.extend(["--mount", f"type=bind,source={report / source},target={destination}" + (",readonly" if readonly else "")])
        if private:
            command.extend(["--mount", f"type=bind,source={report / 'controller'},target=/controller" + ("" if writable_public else ",readonly")])
        if tun:
            command.extend(["--cap-add", "NET_ADMIN", "--device", "/dev/net/tun:/dev/net/tun",
                            "--sysctl", "net.ipv6.conf.all.disable_ipv6=0",
                            "--sysctl", "net.ipv6.conf.default.disable_ipv6=0"])
        command.extend([image, "/fixtures/meshway-packet-fixture", "--log-format", "json", *arguments])
        identifier = run(*command, capture=True).strip()
        containers[role] = identifier
        return identifier

    def execute(role):
        identifier = containers[role]
        with (report / f"{role}.log").open("w") as log:
            result = subprocess.run(["docker", "start", "-a", identifier], cwd=ROOT, text=True,
                                    stdout=log, stderr=subprocess.STDOUT, timeout=180)
        state = json.loads(run("docker", "inspect", identifier, capture=True))[0]["State"]
        return {"attachExit": result.returncode, "exitCode": state["ExitCode"], "oomKilled": state["OOMKilled"]}

    try:
        create("prepare", ["prepare"], private=True, writable_public=True)
        preparation = execute("prepare")
        if preparation["attachExit"] or preparation["exitCode"]:
            raise RuntimeError("fixture TLS preparation failed")
        network = run("docker", "network", "create", "--internal", "--label", label,
                      "meshway-packet-" + identity.lower(), capture=True).strip()
        description = json.loads(run("docker", "network", "inspect", network, capture=True))[0]
        for configuration in description["IPAM"]["Config"]:
            subnet = ipaddress.ip_network(configuration["Subnet"])
            managed = ipaddress.ip_network("100.64.0.0/10" if subnet.version == 4 else "fc00::/7")
            if subnet.overlaps(managed):
                raise RuntimeError("Fixture underlay overlaps its managed address space.")
        create("controller", ["controller"], connected=True, private=True)
        for role in ("a", "r", "b"):
            create(role, ["node", "--role", role], connected=True, tun=role != "r")
        # Four real processes/namespaces. There is no polling of actor progress,
        # host networking, builder privilege, persistent volume or published port.
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            results = dict(zip(("controller", "a", "r", "b"), executor.map(execute, ("controller", "a", "r", "b"))))
        if fingerprint() != before:
            raise RuntimeError("Sources changed while validating candidate binaries.")
        manifest["processes"] = results
        manifest["packetActorsPassed"] = all(not item["attachExit"] and not item["exitCode"] and not item["oomKilled"] for item in results.values())
        metadata.write_text(json.dumps(manifest, indent=2) + "\n")
        for role in ("controller", "a", "r", "b"):
            print(f"{role}: {results[role]}")
            print((report / f"{role}.log").read_text())
        print(report)
        if not manifest["packetActorsPassed"]:
            raise RuntimeError("protected kernel packet verification failed")
    finally:
        for identifier in containers.values():
            run("docker", "rm", "-f", identifier, capture=True)
        if network is not None:
            run("docker", "network", "rm", network, capture=True)
        export_evidence(report, image, platform, "packet", identity)

    # The direct SecureNode fixture cannot detect a daemon-owned pool regression.
    # Require the same socket boundaries through the real daemon as well. Keep
    # its full independent evidence and shutdown assertions, rather than adding
    # another reduced daemon driver with a different lifecycle implementation.
    command = [sys.executable, str(ROOT / "scripts/verify_runtime.py"),
                "--transport", "native-tcp", "--dns-backend", "off"]
    if args.runtime_artifacts:
        command.extend(["--artifacts", str(args.runtime_artifacts)])
    if args.runtime_image_manifest:
        command.extend(["--runtime-image-manifest", str(args.runtime_image_manifest)])
    if args.short:
        command.append("--short")
    if args.report_dir:
        command.extend(["--report-dir", str(report / "daemon" / run_id())])
    with (report / "daemon.log").open("w") as log:
        result = subprocess.run(command, cwd=ROOT, text=True, stdout=log, stderr=subprocess.STDOUT)
    paths = [Path(line) for line in (report / "daemon.log").read_text().splitlines()
             if line.startswith("/") and (Path(line) / "report.json").is_file()]
    if paths:
        daemon_report = paths[-1] / "report.json"
        daemon = json.loads(daemon_report.read_text())
        manifest["daemonReport"] = str(daemon_report)
        manifest["daemonPassed"] = daemon["passed"]
        manifest["passed"] = (result.returncode == 0 and daemon["passed"]
                              and daemon["sourceDigest"] == before and fingerprint() == before)
    manifest["daemonExitCode"] = result.returncode
    metadata.write_text(json.dumps(manifest, indent=2) + "\n")
    print(report)
    if not manifest["passed"]:
        raise RuntimeError("packet daemon MTU conformance failed; inspect daemon.log and its report")


if __name__ == "__main__":
    main()
