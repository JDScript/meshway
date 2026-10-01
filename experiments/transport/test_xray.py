"""Checks for proxy path attribution, credential isolation, and failure semantics."""

import copy
import ipaddress
import json
from pathlib import Path
import signal
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import xray
import xray_node


class XrayHarnessTests(unittest.TestCase):
    def test_routing_denies_other_hosts_ports_and_udp(self):
        for mode in xray.MODES:
            server = xray.configurations(mode, "private-test-id", "192.0.2.2", "198.51.100.3")["LAX"]
            self.assertEqual(server["outbounds"][0]["protocol"], "blackhole")

            def route(host, port, network):
                for rule in server["routing"]["rules"]:
                    if network not in rule["network"].split(","):
                        continue
                    if "ip" in rule and not any(ipaddress.ip_address(host) in ipaddress.ip_network(cidr) for cidr in rule["ip"]):
                        continue
                    if "port" in rule and str(port) not in rule["port"].split(","):
                        continue
                    return rule["outboundTag"]
                return server["outbounds"][0]["tag"]

            for port in (5201, 19090):
                self.assertEqual(route("198.51.100.3", port, "tcp"), "allowed")
                self.assertEqual(route("198.51.100.4", port, "tcp"), "deny")
                self.assertEqual(route("198.51.100.3", port, "udp"), "deny")
            self.assertEqual(route("198.51.100.3", 9000, "tcp"), "deny")

    def test_tls_requires_private_ca_and_san_with_no_insecure_bypass(self):
        configs = xray.configurations("vless-tls", "private-test-id", "192.0.2.2", "198.51.100.3")
        client = configs["NYC"]["outbounds"][0]
        tls = client["streamSettings"]["tlsSettings"]
        self.assertFalse(tls["allowInsecure"])
        self.assertTrue(tls["disableSystemRoot"])
        self.assertEqual(tls["serverName"], xray_node.TLS_NAME)
        self.assertEqual(tls["certificates"][0]["usage"], "verify")
        self.assertEqual(tls["minVersion"], "1.3")
        self.assertFalse(client["mux"]["enabled"])
        self.assertEqual(configs["LAX"]["inbounds"][0]["settings"]["clients"][0]["id"], "private-test-id")

    def test_private_configs_are_per_node_0600_and_removed_after_transfer(self):
        with tempfile.TemporaryDirectory(prefix="xray-test-") as scratch:
            args = xray.arguments(["--output", "/unused/result.json", "--temp-dir", scratch])
            fixture = xray.XrayFixture(args, {"run_id": "xray-test"})
            copies = []

            def copy_file(node, source, destination):
                self.assertEqual(source.stat().st_mode & 0o777, 0o600)
                self.assertEqual(source.parent.stat().st_mode & 0o777, 0o700)
                copies.append((node, json.loads(source.read_text()), destination))

            with patch.object(fixture, "address", return_value="192.0.2.2"), \
                    patch.object(fixture, "copy_file", side_effect=copy_file), \
                    patch.object(fixture, "xhelper", return_value=subprocess.CompletedProcess([], 0)):
                fixture.generate_configs("vless-tcp")
            self.assertEqual([entry[0] for entry in copies], ["NYC", "LAX"])
            client_id = copies[0][1]["outbounds"][0]["settings"]["vnext"][0]["users"][0]["id"]
            self.assertEqual(client_id, copies[1][1]["inbounds"][0]["settings"]["clients"][0]["id"])
            self.assertNotIn(client_id, json.dumps(fixture.result))
            self.assertEqual(list(Path(scratch).iterdir()), [])

    def test_iperf_peer_checks_catch_one_bypassing_stream_or_control(self):
        client = {"start": {"connecting_to": {"host": "127.0.0.1", "port": 15201}, "connected": [
            {"local_host": "127.0.0.1", "remote_host": "127.0.0.1", "remote_port": 15201} for _ in range(4)]}}
        server = {"start": {"accepted_connection": {"host": "198.51.100.2"}, "connected": [
            {"local_host": "198.51.100.3", "local_port": 5201, "remote_host": "198.51.100.2"} for _ in range(4)]}}

        def passed(c, s):
            return xray.iperf_path_check(c, s, 4, "198.51.100.2", "198.51.100.3")["passed"]

        self.assertTrue(passed(client, server))
        bypass = copy.deepcopy(server)
        bypass["start"]["connected"][2]["remote_host"] = "192.0.2.3"
        self.assertFalse(passed(client, bypass))
        bypass = copy.deepcopy(server)
        bypass["start"]["accepted_connection"]["host"] = "192.0.2.3"
        self.assertFalse(passed(client, bypass))
        bypass = copy.deepcopy(client)
        bypass["start"]["connected"].pop()
        self.assertFalse(passed(bypass, server))

    def test_iperf_commands_split_loopback_client_from_actual_server_bind(self):
        args = xray.arguments(["--output", "/unused/result.json", "--seconds", "5", "--omit", "2"])
        server, client, _ = xray.case_commands(args, "198.51.100.3", 4, True, "case")
        self.assertEqual(server[server.index("--bind") + 1], "198.51.100.3")
        for option in ("--client", "--bind"):
            self.assertEqual(client[client.index(option) + 1], "127.0.0.1")
        self.assertEqual(client[client.index("--port") + 1], "15201")
        self.assertEqual(client[client.index("--omit") + 1], "2")
        self.assertIn("--reverse", client)

    def test_fault_check_accepts_connection_errors_but_not_cli_errors_or_success(self):
        for code, stdout, expected in (
            (1, '{"error":"unable to receive control message: Connection reset by peer"}', True),
            (-signal.SIGPIPE, "", True),
            (1, "", False),  # In particular, invalid flags are not path-block evidence.
            (0, '{"end":{}}', False),
        ):
            with patch.object(xray_node.subprocess, "run", return_value=subprocess.CompletedProcess([], code, stdout, "")):
                self.assertEqual(xray_node.fault_client()["blocked"], expected)
        with patch.object(xray_node.subprocess, "run", side_effect=subprocess.TimeoutExpired("iperf3", 5)):
            self.assertTrue(xray_node.fault_client()["child_killed_and_reaped"])

    def test_stale_pid_does_not_stop_an_unrelated_process(self):
        with tempfile.TemporaryDirectory(prefix="xray-test-") as scratch:
            root = Path(scratch)
            (root / "xray-pid.json").write_text(json.dumps({"pid": 42, "start_ticks": 100}))
            with patch.object(xray_node, "ROOT", root), \
                    patch.object(xray_node, "process_identity", return_value={"state": "S", "start_ticks": 200}):
                self.assertFalse(xray_node.state()["running"])

    def test_tcp_namespace_counters_preserve_gauge_deltas(self):
        before = {"LAX": {"files": {"snmp": "Tcp: RetransSegs CurrEstab\nTcp: 2 5\n", "netstat": "TcpExt: TCPTimeouts\nTcpExt: 1\n"}}}
        after = {"LAX": {"files": {"snmp": "Tcp: RetransSegs CurrEstab\nTcp: 7 0\n", "netstat": "TcpExt: TCPTimeouts\nTcpExt: 3\n"}}}
        self.assertEqual(xray.socket_counter_delta(before, after)["tcp_deltas"]["LAX"],
                         {"Tcp.RetransSegs": 5, "Tcp.CurrEstab": -5, "TcpExt.TCPTimeouts": 2})


if __name__ == "__main__":
    unittest.main()
