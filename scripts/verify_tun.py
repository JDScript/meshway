#!/usr/bin/env python3
"""Explicit Linux kernel TUN verification: host build, isolated runtime only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

from node_lab import ROOT, base_image, check_elf, docker_target, run, sha256
from validation_support import build_lock, cargo_target, fixture_options, report_directory, save_artifact, import_artifact, bind_access_options


def fingerprint():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    paths.extend(path for path in (ROOT / "crates/meshway-tun").rglob("*")
                 if path.is_file() and (path.suffix == ".rs" or path.name == "Cargo.toml"))
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0")
        digest.update(path.read_bytes() + b"\0")
    return digest.hexdigest()


def pinned_image(platform, directory):
    # Kernel/packet fixtures need no package installation or persistent-lab
    # deployment. The locked OS inputs own the one Ubuntu base declaration.
    return base_image()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    fixture_options(parser)
    args = parser.parse_args()
    os.umask(0o077)
    if shutil.disk_usage(ROOT).free < 50 * 1024 ** 3:
        raise SystemExit("Linux build needs at least 50 GiB of free disk space.")
    target, platform, machine = docker_target()
    report = report_directory("tun", args.report_dir)
    identity = report.name
    before = fingerprint()
    env = dict(os.environ, CARGO_TARGET_DIR=str(cargo_target()), CARGO_INCREMENTAL="0")
    binary = report / "linux-tun-tests"
    if args.artifacts:
        import_artifact(args.artifacts, binary, before, target, platform, machine)
    else:
        with build_lock():
            run("rustup", "target", "add", target)
            result = run("cargo", "zigbuild", "--target", target, "--tests", "--all-features",
                         "--locked", "-p", "meshway-tun", "--message-format=json", capture=True, env=env)
            artifacts = [json.loads(line) for line in result.splitlines() if line.startswith("{")]
            executables = [Path(item["executable"]) for item in artifacts
                           if item.get("reason") == "compiler-artifact" and item.get("executable")
                           and item["target"]["name"] == "linux" and item["profile"].get("test")]
            if len(executables) != 1 or fingerprint() != before:
                raise SystemExit("Expected one unchanged-source Linux TUN test artifact.")
            shutil.copyfile(executables[0], binary)
            binary.chmod(0o500)
    check_elf(binary, machine)
    save_artifact(report, binary, before, target, platform)
    if args.build_only:
        print(report)
        return
    image = pinned_image(platform, report)
    command = ["docker", "create", "--name", "meshway-tun-check-" + identity.lower(),
               "--label", "io.meshway.tun-fixture=" + identity,
               "--platform", platform, "--network", "none", "--read-only",
               "--cap-drop", "ALL", "--cap-add", "NET_ADMIN",
               "--security-opt", "no-new-privileges", "--device", "/dev/net/tun:/dev/net/tun",
               "--cpus", "2", "--memory", "512m", "--pids-limit", "128",
               "--sysctl", "net.ipv6.conf.all.disable_ipv6=0",
               "--sysctl", "net.ipv6.conf.default.disable_ipv6=0",
               "--env", "MESHWAY_TUN_TEST_ISOLATED=1",
               "--mount", f"type=bind,source={report},target=/fixtures,readonly", *bind_access_options(), image,
               "/fixtures/linux-tun-tests", "--ignored", "--exact",
               "owned_devices_ipv4_ipv6_packet_io", "--nocapture"]
    manifest = {"revision": run("git", "rev-parse", "HEAD", capture=True).strip(),
                "dirty": bool(run("git", "status", "--porcelain", capture=True).strip()),
                "sourceDigest": before, "target": target, "platform": platform,
                "image": image, "sha256": sha256(binary), "command": command, "passed": False}
    metadata = report / "report.json"
    metadata.write_text(json.dumps(manifest, indent=2) + "\n")
    container = run(*command, capture=True).strip()
    try:
        with (report / "kernel.log").open("w") as log:
            completed = subprocess.run(["docker", "start", "-a", container], cwd=ROOT,
                                       stdout=log, stderr=subprocess.STDOUT, text=True, timeout=60)
        status = json.loads(run("docker", "inspect", container, capture=True))[0]["State"]
        manifest["exitCode"] = status["ExitCode"]
        manifest["passed"] = completed.returncode == 0 and status["ExitCode"] == 0
        metadata.write_text(json.dumps(manifest, indent=2) + "\n")
        print((report / "kernel.log").read_text())
        print(report)
        if not manifest["passed"]:
            raise SystemExit("Linux TUN verification failed; see the report.")
    finally:
        run("docker", "rm", "-f", container, capture=True)


if __name__ == "__main__":
    main()
