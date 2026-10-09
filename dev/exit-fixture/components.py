"""Same-build production driver assertions which are not observable via IPC.

These are explicitly component evidence, never multi-process packet acceptance.
No NAT, permit, flow or authority implementation lives in this helper.
"""
import ipaddress
import json
import subprocess

from node_lab import run
from validation_support import bind_access_options


CASES = {
    "driver": "daemon_exit_driver_real_nat_renew_confirm_failure_and_retirement",
    "handoff": "daemon_exit_control_handoff_preserves_projection",
    "role": "daemon_exit_role_applied_probe_failure_and_instance_replacement",
    "dns-handoff": "daemon_exit_dns_role_refresh_preserves_applied",
    "unavailable": "daemon_exit_role_applied_probe_failure_and_instance_replacement",
}


def execute(report, image, platform, identity):
    directory = report / "components"
    directory.mkdir()
    label = "io.meshway.runtime-fixture=" + identity
    network = None
    containers = []
    results = {}
    commands = []
    service = ("import socket\ns=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)\n"
               "s.bind(('0.0.0.0',18080))\nwhile True:\n data,peer=s.recvfrom(65535)\n"
               " print(peer,data.hex(),flush=True)\n s.sendto(data,peer)\n")
    try:
        network = run("docker", "network", "create", "--internal", "--label", label,
                      "meshway-exit-components-" + identity.lower(), capture=True).strip()
        inspection = json.loads(run("docker", "network", "inspect", network, capture=True))[0]
        subnet = ipaddress.ip_network(inspection["IPAM"]["Config"][0]["Subnet"])
        if not inspection["Internal"] or subnet.num_addresses < 16:
            raise RuntimeError("component fixture requires an internal /28 or larger")
        remote = str(subnet.network_address + 2)
        server = run("docker", "create", "--pull", "never", "--label", label, "--network", network,
                     "--ip", remote, "--cap-drop", "ALL", "--read-only", "--security-opt", "no-new-privileges",
                     "--cpus", "1", "--memory", "128m", "--pids-limit", "32", "--entrypoint", "python3",
                     image, "-u", "-c", service, capture=True).strip()
        containers.append(server)
        run("docker", "start", server, capture=True)
        for index, (name, test) in enumerate(CASES.items()):
            command = ["docker", "create", "--pull", "never", "--label", label, "--platform", platform,
                       "--network", network, "--ip", str(subnet.network_address + 3 + index),
                       "--read-only", "--cap-drop", "ALL", "--cap-add", "NET_ADMIN", *bind_access_options(),
                       "--device", "/dev/net/tun", "--security-opt", "no-new-privileges", "--cpus", "2",
                       "--memory", "1g", "--pids-limit", "128", "--tmpfs", "/state:mode=700",
                       "--tmpfs", "/run:mode=755", "--env", "TMPDIR=/state", "--env", "RUST_MIN_STACK=8388608",
                       "--env", "MESHWAY_UNDERLAY_ISOLATED=1", "--env", "MESHWAY_X4_EXTERNAL=" + remote + ":18080",
                       "--env", "MESHWAY_X4_CLI=/fixtures/meshway",
                       "--sysctl", "net.ipv4.ip_forward=" + ("0" if name == "unavailable" else "1"),
                       "--sysctl", "net.ipv4.conf.all.rp_filter=0", "--sysctl", "net.ipv4.conf.default.rp_filter=0",
                       "--sysctl", "net.netfilter.nf_conntrack_acct=0",
                       "--mount", f"type=bind,source={report / 'bin'},target=/fixtures,readonly", "--entrypoint", ""]
            if name == "unavailable": command += ["--env", "MESHWAY_X4_UNAVAILABLE=1"]
            command += [image, "/bin/sh", "-ec", 'umask 077; exec "$@"', "exit-components",
                        "/fixtures/meshway-daemon-platform-tests", "--exact", "runtime::network::exit::tests::" + test,
                        "--ignored", "--nocapture", "--test-threads=1"]
            commands.append(command)
            identifier = run(*command, capture=True).strip()
            containers.append(identifier)
            with (directory / (name + ".log")).open("w") as log:
                result = subprocess.run(["docker", "start", "-a", identifier], stdout=log, stderr=subprocess.STDOUT,
                                        text=True, timeout=150, close_fds=True)
            state = json.loads(run("docker", "inspect", identifier, capture=True))[0]["State"]
            results[name] = {"test": test, "exitCode": result.returncode, "state": state,
                             "passed": result.returncode == 0 and state["ExitCode"] == 0 and not state["OOMKilled"]
                             and "1 passed; 0 failed;" in (directory / (name + ".log")).read_text()}
        (directory / "external.log").write_text(run("docker", "logs", server, capture=True))
        driver_source = str(subnet.network_address + 3)
        permitted = [line for line in (directory / "external.log").read_text().splitlines() if line.startswith("('" + driver_source + "',")]
        results["external-log"] = {"passed": len(permitted) == 3, "driverPackets": len(permitted)}
    finally:
        for identifier in reversed(containers):
            inspection = json.loads(run("docker", "inspect", identifier, capture=True))[0]
            if inspection["Config"]["Labels"].get("io.meshway.runtime-fixture") != identity:
                raise RuntimeError("component container ownership changed")
            run("docker", "rm", "-f", identifier, capture=True)
        if network:
            run("docker", "network", "rm", network, capture=True)
        (directory / "results.json").write_text(json.dumps({"scope": "component", "commands": commands, "checks": results}, indent=2))
    if len(results) != len(CASES) + 1 or not all(item["passed"] for item in results.values()):
        raise RuntimeError("exit component assertions failed")
    return results
