"""Same-build production exit component assertions not observable via IPC.

These are explicitly component evidence, never multi-process packet acceptance:
the main actors prove installation, NAT and traffic lifecycle themselves.
No NAT, permit, flow or authority implementation lives in this helper.
"""
import ipaddress
import json
import subprocess

from node_lab import run
from validation_support import bind_access_options


# The netmap exit role owner: renewal and unchanged-serial apply keep it, Access
# narrowing admits no new projection, withdrawal ends it; a flow refused at
# projection capacity does not stop it; its own failures stop it alone, and a failed cleanup is retried. Then the role's nft transaction and the
# live exit-prerequisite probe, in the same build.
CASES = {
    "role": ("meshway-underlay-netmap-tests",
             "netmap_exit_role_survives_renewal_and_unchanged_serial_and_ends_on_withdrawal"),
    "capacity": ("meshway-underlay-netmap-tests",
                 "netmap_exit_projection_capacity_refuses_one_flow_and_keeps_the_role"),
    "round": ("meshway-underlay-netmap-tests",
              "netmap_exit_projection_round_acknowledges_each_flow_by_itself"),
    # OPS8: a re-created echo conntrack entry is no ownership conflict; a
    # deleted exit-owned route stops the role alone, which reinstalls.
    "recovery": ("meshway-underlay-netmap-tests",
                 "exit_recovery::netmap_exit_echo_reconnect_and_route_loss_stop_only_the_role"),
    # X11: a failed cleanup keeps the resources and is retried from what the
    # kernel then holds; the role installs again once it completed. A failed
    # audit read is unknown, never a change.
    "cleanup": ("meshway-underlay-netmap-tests",
                "exit_recovery::netmap_exit_failed_cleanup_is_retried_and_the_role_installs_again"),
    "audit": ("meshway-underlay-kernel-tests",
              "linux::kernel::exit::tests::cleanup::exit_audit_read_failures_are_unknown_and_a_failed_cleanup_completes_on_retry"),
    # X12: a failed kernel change proves neither outcome; the owner reads
    # back and completes it from the target or the start (each change path
    # injected applied-then-failed and failed-unseen), or fails closed on a
    # read-back that is neither.
    "changes-exit": ("meshway-underlay-kernel-tests",
                     "linux::kernel::exit::tests::changes::failed_exit_changes_are_read_back_and_completed"),
    # F6a B6: an aged quarantine deletion applied but unanswered completes
    # from the read-back instead of deleting nothing until the role stops.
    "quarantine": ("meshway-underlay-kernel-tests",
                   "linux::kernel::exit::tests::changes::a_lost_quarantine_expiry_is_completed_from_the_read_back"),
    "changes-egress": ("meshway-underlay-kernel-tests",
                       "linux::kernel::egress::tests::changes::failed_capture_changes_are_read_back_and_completed"),
    "nft": ("meshway-underlay-kernel-tests",
            "linux::kernel::exit::nft::tests::exit_nft_atomic_batch_readback_and_failed_transaction_leave_no_table"),
    "probe": ("meshway-underlay-kernel-tests",
              "linux::kernel::exit::live_probe::tests::live_probe_open_host_reports_ipv4_ready_without_external_change"),
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
        for index, (name, (binary, test)) in enumerate(CASES.items()):
            command = ["docker", "create", "--pull", "never", "--label", label, "--platform", platform,
                       "--network", network, "--ip", str(subnet.network_address + 3 + index),
                       "--read-only", "--cap-drop", "ALL", "--cap-add", "NET_ADMIN", *bind_access_options(),
                       "--device", "/dev/net/tun", "--security-opt", "no-new-privileges", "--cpus", "2",
                       "--memory", "1g", "--pids-limit", "128", "--tmpfs", "/state:mode=700",
                       "--tmpfs", "/run:mode=755", "--env", "TMPDIR=/state", "--env", "RUST_MIN_STACK=8388608",
                       "--env", "MESHWAY_UNDERLAY_ISOLATED=1", "--env", "MESHWAY_X4_EXTERNAL=" + remote + ":18080",
                       "--env", "MESHWAY_X4_CLI=/fixtures/meshway",
                       "--sysctl", "net.ipv4.ip_forward=1",
                       "--sysctl", "net.ipv4.conf.all.rp_filter=0", "--sysctl", "net.ipv4.conf.default.rp_filter=0",
                       "--sysctl", "net.netfilter.nf_conntrack_acct=0",
                       "--mount", f"type=bind,source={report / 'bin'},target=/fixtures,readonly", "--entrypoint", ""]
            command += [image, "/bin/sh", "-ec", 'umask 077; exec "$@"', "exit-components",
                        "/fixtures/" + binary, "--exact", test,
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
    finally:
        for identifier in reversed(containers):
            inspection = json.loads(run("docker", "inspect", identifier, capture=True))[0]
            if inspection["Config"]["Labels"].get("io.meshway.runtime-fixture") != identity:
                raise RuntimeError("component container ownership changed")
            run("docker", "rm", "-f", identifier, capture=True)
        if network:
            run("docker", "network", "rm", network, capture=True)
        (directory / "results.json").write_text(json.dumps({"scope": "component", "commands": commands, "checks": results}, indent=2))
    if len(results) != len(CASES) or not all(item["passed"] for item in results.values()):
        raise RuntimeError("exit component assertions failed")
    return results
