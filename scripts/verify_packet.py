#!/usr/bin/env python3
"""Protected TUN conformance in four disposable, isolated Linux namespaces."""
import concurrent.futures
import datetime
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import shutil
import subprocess
import uuid

from node_lab import ROOT, check_elf, docker_target, run, sha256
from verify_tun import pinned_image


def fingerprint():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    for directory in ("crates", "apps/meshway-server", "tools/meshway-packet-fixture"):
        paths.extend(p for p in (ROOT / directory).rglob("*")
                     if p.is_file() and (p.suffix == ".rs" or p.name == "Cargo.toml"))
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def main():
    os.umask(0o077)
    if shutil.disk_usage(ROOT).free < 50 * 1024 ** 3:
        raise SystemExit("Linux build needs at least 50 GiB of free disk space.")
    target, platform, machine = docker_target()
    common = Path(run("git", "rev-parse", "--git-common-dir", capture=True).strip())
    repository = (ROOT / common).resolve().parent
    identity = datetime.datetime.now(datetime.UTC).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    report = repository / ".meshway/validation/packet" / identity
    for directory in ("bin", "public", "controller", "signals"):
        (report / directory).mkdir(parents=True)
    before = fingerprint()
    env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "target/node-kernel-tests"), CARGO_INCREMENTAL="0")
    run("rustup", "target", "add", target)
    run("cargo", "zigbuild", "--target", target, "--locked", "--all-features",
        "-p", "meshway-packet-fixture", "--bin", "meshway-packet-fixture", env=env)
    if fingerprint() != before:
        raise SystemExit("Sources changed during the build; repeat before verification.")
    binary = report / "bin/meshway-packet-fixture"
    shutil.copyfile(Path(env["CARGO_TARGET_DIR"]) / target / "debug/meshway-packet-fixture", binary)
    binary.chmod(0o500)
    check_elf(binary, machine)
    image = pinned_image(platform, report / "bin")
    label = "io.meshway.packet-fixture=" + identity
    containers = {}
    network = None
    manifest = {"revision": run("git", "rev-parse", "HEAD", capture=True).strip(),
                "dirty": bool(run("git", "status", "--porcelain", capture=True).strip()),
                "sourceDigest": before, "target": target, "platform": platform,
                "image": image, "sha256": sha256(binary), "passed": False,
                "scope": "real HTTPS control and IPv4/IPv6 UDP OS applications over TUN, Noise and a mandatory relay on native TCP"}
    metadata = report / "report.json"
    metadata.write_text(json.dumps(manifest, indent=2) + "\n")

    def create(role, arguments, connected=False, private=False, tun=False, writable_public=False):
        command = ["docker", "create", "--name", f"meshway-packet-{identity.lower()}-{role}",
                   "--label", label, "--platform", platform, "--read-only", "--cap-drop", "ALL",
                   "--security-opt", "no-new-privileges", "--cpus", "2", "--memory", "768m",
                   "--pids-limit", "128", "--tmpfs", "/state:rw,nosuid,nodev,mode=700"]
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
        manifest["processes"] = results
        manifest["passed"] = all(not item["attachExit"] and not item["exitCode"] and not item["oomKilled"] for item in results.values())
        metadata.write_text(json.dumps(manifest, indent=2) + "\n")
        for role in ("controller", "a", "r", "b"):
            print(f"{role}: {results[role]}")
            print((report / f"{role}.log").read_text())
        print(report)
        if not manifest["passed"]:
            raise RuntimeError("protected kernel packet verification failed")
    finally:
        for identifier in containers.values():
            run("docker", "rm", "-f", identifier, capture=True)
        if network is not None:
            run("docker", "network", "rm", network, capture=True)


if __name__ == "__main__":
    main()
