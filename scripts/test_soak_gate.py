"""Offline tests of the soak collector's forced-command gate (scripts/soak/gate.py) and of what install.sh renders:
the allow table per host, injection attempts, the fixed argument arrays (no shell for the requested command),
fault forms only where the schedule names them, the authorized_keys line and the crontab block."""
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent / "soak"))
import common  # noqa: E402
import gate  # noqa: E402
import render  # noqa: E402

CURSOR = "s=" + "a" * 32 + ";i=1b54e;b=" + "b" * 32 + ";m=2e2c1e3b60;t=65e7f2d765c8f;x=f4c4df18d2c4ebb2"
CONFIG = {
    "t0": "2026-11-02T00:00:00Z", "days": 7,
    "hosts": {"kr": {"kind": "local", "sudo": True, "docker_sudo": True},
              "jp": {"kind": "ssh", "target": "ubuntu@jp.invalid", "sudo": True},
              "it7": {"kind": "ssh", "target": "root@it7.invalid", "sudo": True},
              "planck": {"kind": "ssh", "target": "u@planck.invalid", "sudo": True},
              "hk": {"kind": "ssh", "target": "root@hk.invalid", "sudo": True}},
    "nodes": {"it7": {"host": "it7", "unit": "meshwayd", "mesh_name": "us-lax-it7"},
              "jp": {"host": "jp", "unit": "meshwayd", "mesh_name": "jp-tyo-arm64"},
              "kr": {"host": "kr", "container": "meshway-kr", "mesh_name": "kr-icn-arm64"},
              "planck": {"host": "planck", "container": "meshway-planck", "mesh_name": "meshway-planck"},
              "hk": {"host": "hk", "container": "meshway-hk", "mesh_name": "cn-hkg-dmit"}},
    "server": {"health_url": "https://server.invalid/api/v1/auth/options", "host": "planck",
               "container": "meshway-prod-server", "image": "sha256:" + "7" * 64, "release": "/srv/release/server",
               "data": "/srv/server", "user": "10480:10480", "upgrade_script": "/srv/server-upgrade.sh",
               "stage": "/nonexistent/soak-stage", "asset": "meshway-server-x86_64-unknown-linux-musl.tar.gz",
               "release_url": "https://github.com/JDScript/meshway/releases/download"},
    "edges": [["it7", "jp"], ["it7", "kr"], ["jp", "kr"], ["kr", "hk"], ["planck", "hk"]],
    "throughput": {"client": "jp", "server": "kr", "target": "100.72.0.5", "port": 5299},
    "faults": [
        {"id": "d2", "day": 2, "at": "10:00", "action": "server_upgrade", "version": "v0.1.0-alpha.23"},
        {"id": "d3", "day": 3, "at": "10:00", "action": "server_stop"},
        {"id": "d4", "day": 4, "at": "10:00", "action": "server_rotate_key"},
        {"id": "d5a", "day": 5, "at": "10:00", "action": "systemd_restart", "node": "jp"},
        {"id": "d5b", "day": 5, "at": "12:00", "action": "kill9", "node": "kr"},
        {"id": "d5c", "day": 5, "at": "14:00", "action": "docker_restart", "node": "hk"},
        {"id": "d5d", "day": 5, "at": "16:00", "action": "systemd_restart", "node": "it7"},
        {"id": "d6a", "day": 6, "at": "10:00", "action": "nft_drop", "node": "planck", "match": "udp"},
        {"id": "d6b", "day": 6, "at": "12:00", "action": "nft_drop", "node": "hk", "match": "tcp443"},
        {"id": "d6c", "day": 6, "at": "14:00", "action": "exit_profile", "node": "hk", "profile": "us-exit",
         "url": "https://www.example.com/"}],
}
INJECTIONS = [
    "", "   ", "foo", "status", "status it7; id", "status it7 && id", "status $(id)", "status `id`",
    "status it7\nstatus it7", "status it7 extra", "status 'it7;id'", "status ../it7", "status 'it7 '",
    "resources it7 60;id", "resources it7 999", "resources it7 -1", "resources it7 6e1",
    "journal it7 --after-cursor 's=x;rm -rf /'", "journal it7 --after-cursor", "journal it7 --since 17935776000",
    "journal it7 --since 1793577600 --after-cursor " + CURSOR, "journal it7 --until 1793577600",
    "ping it7 'jp-tyo-arm64 && id'", "ping it7 example.com", "ping it7 jp-tyo-arm64 -c 1000",
    "status it7 \"$(reboot)\"", "status it7 'unterminated", "self-test now",
]


def gates():
    config = common.Config(json.loads(json.dumps(CONFIG)), Path("/nonexistent/config.json"))
    return config, {host: gate.Gate(common.gate_spec(config, host)) for host in config.hosts}


def argv(steps):
    return [step.argv if isinstance(step, gate.Step) else (step.name, step.args) for step in steps]


class Specs(unittest.TestCase):
    def test_each_host_is_allowed_exactly_the_faults_the_schedule_names_for_it(self):
        _, by_host = gates()
        self.assertEqual({h: g.faults for h, g in by_host.items()}, {
            "it7": {"it7": ["restart"]}, "jp": {"jp": ["restart"]}, "kr": {"kr": ["kill9"]},
            "planck": {"server": ["server-upgrade", "server-stop", "server-rotate-key"], "planck": ["nft-drop-udp"]},
            "hk": {"hk": ["docker-restart", "nft-drop-tcp443", "exit"]}})
        self.assertEqual(by_host["kr"].docker, ["sudo", "-n", "docker"])
        self.assertEqual(by_host["it7"].nodes["it7"]["ping"], ["jp-tyo-arm64", "kr-icn-arm64"])
        self.assertEqual(by_host["kr"].nodes["kr"]["iperf_server"], [5299])
        self.assertEqual(by_host["jp"].nodes["jp"]["iperf_client"], {"targets": ["100.72.0.5"], "ports": [5299]})
        self.assertEqual(by_host["hk"].nodes["hk"]["exit"], {"profiles": ["us-exit"], "url": "https://www.example.com/"})
        self.assertNotIn("server", by_host["hk"].spec)


class AllowTable(unittest.TestCase):
    ALLOWED = {
        "it7": [("status it7", ["sudo", "-n", "sh", "-c"]), ("resources it7 60", ["sudo", "-n", "sh", "-c"]),
                ("inspect it7", ["systemctl", "show", "-p", "MainPID,Restart,NRestarts,ExecMainStartTimestamp",
                                 "meshwayd"]),
                (f"journal it7 --after-cursor {CURSOR}", ["sudo", "-n", "journalctl", "-u", "meshwayd", "-o", "json",
                                                          "--no-pager", "--after-cursor", CURSOR]),
                ("journal it7 --since 1793577600", ["sudo", "-n", "journalctl", "-u", "meshwayd", "-o", "json",
                                                    "--no-pager", "--since", "@1793577600"]),
                ("ping it7 jp-tyo-arm64", ["sudo", "-n", "meshway", "--json", "ping", "-c", "10", "jp-tyo-arm64"]),
                ("restart it7", ["sudo", "-n", "systemctl", "restart", "meshwayd"])],
        "jp": [("restart jp", ["sudo", "-n", "systemctl", "restart", "meshwayd"]),
               ("iperf-client jp 100.72.0.5 5299 10 50",
                ["sudo", "-n", "iperf3", "-c", "100.72.0.5", "-p", "5299", "-t", "10", "-b", "50M", "-J"])],
        "kr": [("status kr", ["sudo", "-n", "docker", "exec", "meshway-kr", "sh", "-c"]),
               ("iperf-server kr 5299 60", ["sudo", "-n", "docker", "exec", "meshway-kr", "timeout", "60", "iperf3", "-s",
                                            "-1", "-p", "5299"]),
               ("docker-logs kr 1793577600.000000000 1793581200.000000000",
                ["sudo", "-n", "docker", "logs", "--timestamps", "--since", "1793577600.000000000", "--until",
                 "1793581200.000000000", "meshway-kr"]),
               ("ping kr cn-hkg-dmit", ["sudo", "-n", "docker", "exec", "meshway-kr", "meshway", "--json", "ping", "-c",
                                        "10", "cn-hkg-dmit"])],
        "planck": [("inspect server", ["docker", "inspect", "-f"]),
                   ("docker-logs server 1793577600.000000000 1793581200.000000000", ["docker", "logs", "--timestamps"]),
                   ("server-stop", ["docker", "stop", "-t", "30", "meshway-prod-server"]),
                   ("server-start", ["docker", "start", "meshway-prod-server"]),
                   ("nft-clear planck", ["docker", "exec", "meshway-planck", "nft", "delete", "table", "inet",
                                         "meshway_soak_fault"])],
        "hk": [("docker-restart hk", ["docker", "restart", "-t", "30", "meshway-hk"]),
               ("exit-select hk us-exit", ["docker", "exec", "meshway-hk", "meshway", "--json", "exit", "us-exit"]),
               ("exit-default hk", ["docker", "exec", "meshway-hk", "meshway", "--json", "exit", "--default"]),
               ("curl-loop hk 600", ["docker", "exec", "meshway-hk", "sh", "-c"])],
    }
    REFUSED = {
        "it7": ["kill9 it7", "docker-restart it7", "exit-select it7 us-exit", "curl-loop it7 600", "nft-drop it7 udp 60",
                "server-stop", "server-upgrade v0.1.0-alpha.23", "iperf-client it7 100.72.0.5 5299 10 50",
                "iperf-server it7 5299 60", "docker-logs it7 1793577600.000000000 1793581200.000000000",
                "status jp", "restart jp", "ping it7 cn-hkg-dmit"],
        "jp": ["kill9 jp", "exit-select jp us-exit", "iperf-client jp 100.72.0.6 5299 10 50",
               "iperf-client jp 100.72.0.5 22 10 50", "iperf-client jp 100.72.0.5 5299 600 50",
               "iperf-client jp 100.72.0.5 5299 10 1000"],
        "kr": ["restart kr", "docker-restart kr", "journal kr --since 1793577600", "iperf-server kr 22 60",
               "nft-drop kr udp 60", "inspect server"],
        "planck": ["nft-drop planck tcp443 60", "nft-drop planck udp 99999", "nft-drop planck udp 5", "kill9 planck",
                   "server-upgrade v0.1", "server-upgrade 'v0.1.0-alpha.23;id'", "server-upgrade v0.1.0-alpha.23 x",
                   "server-upgrade latest", "server-upgrade v0.1.0-alpha.23/../../x", "exit-select planck us-exit",
                   "docker-logs server 1793581200.000000000 1793577600.000000000"],
        "hk": ["nft-drop hk udp 60", "exit-select hk mesh", "exit-select hk 'us-exit;id'", "curl-loop hk 99999",
               "server-start", "restart hk"],
    }

    def test_allowed_forms_become_fixed_argument_arrays(self):
        _, by_host = gates()
        for host, cases in self.ALLOWED.items():
            for command, prefix in cases:
                with self.subTest(host=host, command=command):
                    steps = by_host[host].plan(command)
                    self.assertEqual(steps[0].argv[:len(prefix)], prefix)

    def test_forms_outside_the_host_or_its_schedule_are_refused(self):
        _, by_host = gates()
        for host, commands in self.REFUSED.items():
            for command in commands:
                with self.subTest(host=host, command=command), self.assertRaises(gate.Refused):
                    by_host[host].plan(command)

    def test_injection_attempts_are_refused_on_every_host(self):
        _, by_host = gates()
        for host, keeper in by_host.items():
            for command in INJECTIONS + [c.replace("it7", host) for c in INJECTIONS]:
                with self.subTest(host=host, command=command), self.assertRaises(gate.Refused):
                    keeper.plan(command)

    def test_the_requested_words_are_never_handed_to_a_shell(self):
        _, by_host = gates()
        for host, cases in self.ALLOWED.items():
            for command, _ in cases:
                for step in by_host[host].plan(command):
                    if isinstance(step, gate.Step) and "sh" in step.argv and "-c" in step.argv:
                        script = step.argv[step.argv.index("-c") + 1]
                        self.assertNotIn(command, script)  # only the gate's fixed probe scripts run in sh

    def test_fault_plans(self):
        _, by_host = gates()
        udp = by_host["planck"].plan("nft-drop planck udp 300")
        self.assertEqual(udp[0].argv[:5], ["docker", "exec", "-d", "meshway-planck", "sh"])
        self.assertIn("sleep 420; nft delete table inet meshway_soak_fault", udp[0].argv[-1])
        self.assertEqual(udp[1].argv, ["docker", "exec", "-i", "meshway-planck", "nft", "-f", "-"])
        self.assertEqual(udp[1].stdin.count("meta l4proto udp drop"), 3)
        self.assertIn("iif lo accept", udp[1].stdin)
        tcp = by_host["hk"].plan("nft-drop hk tcp443 600")[1].stdin
        self.assertIn("table inet meshway_soak_fault", tcp)
        self.assertIn("hook output", tcp)
        self.assertIn("tcp dport 443 drop", tcp)
        self.assertNotIn("udp", tcp)
        self.assertNotIn("daddr", tcp)
        rotate = by_host["planck"].plan("server-rotate-key")
        self.assertEqual([s.argv[1] for s in rotate], ["stop", "run", "start"])
        self.assertEqual([s.always for s in rotate], [False, False, True])
        self.assertIn("rotate-netmap-key", rotate[1].argv)
        self.assertIn("--network", rotate[1].argv)
        upgrade = by_host["planck"].plan("server-upgrade v0.1.0-alpha.23")
        self.assertEqual(upgrade[1].argv[-1], "https://github.com/JDScript/meshway/releases/download/v0.1.0-alpha.23/"
                                              "meshway-server-x86_64-unknown-linux-musl.tar.gz")
        self.assertEqual(upgrade[-1].argv[:4], ["sudo", "-n", "sh", "/srv/server-upgrade.sh"])
        self.assertEqual(argv(by_host["kr"].plan("kill9 kr")), [("kill9", ("kr",))])


class Main(unittest.TestCase):
    def spec_file(self, directory, host):
        config, _ = gates()
        path = Path(directory) / f"{host}.json"
        path.write_text(json.dumps(common.gate_spec(config, host)))
        return str(path)

    def test_refusals_exit_126_and_go_to_syslog_and_stderr(self):
        with tempfile.TemporaryDirectory() as directory:
            spec = self.spec_file(directory, "it7")
            logged = []
            with mock.patch.object(gate, "log", side_effect=lambda level, message: logged.append((level, message))), \
                    mock.patch.dict(os.environ, {"SSH_ORIGINAL_COMMAND": "status it7; reboot"}), \
                    mock.patch.object(os, "execvp") as execvp:
                err = io.StringIO()
                with redirect_stderr(err):
                    self.assertEqual(gate.main(["--spec", spec, "it7"]), 126)
                    self.assertEqual(gate.main(["--spec", spec, "jp"]), 126)  # spec of another host
            execvp.assert_not_called()
            self.assertEqual([level for level, _ in logged], [4, 4])
            self.assertIn("refused on it7", err.getvalue())
            self.assertIn("spec is for 'it7', not 'jp'", logged[1][1])

    def test_an_allowed_form_execs_its_argument_array(self):
        with tempfile.TemporaryDirectory() as directory:
            spec = self.spec_file(directory, "it7")
            with mock.patch.object(gate, "log"), mock.patch.dict(os.environ, {"SSH_ORIGINAL_COMMAND": "restart it7"}), \
                    mock.patch.object(os, "execvp") as execvp:
                gate.main(["--spec", spec, "it7"])
            execvp.assert_called_once_with("sudo", ["sudo", "-n", "systemctl", "restart", "meshwayd"])

    def test_self_test_reports_the_spec(self):
        with tempfile.TemporaryDirectory() as directory:
            spec = self.spec_file(directory, "hk")
            out = io.StringIO()
            with mock.patch.object(gate, "log"), redirect_stdout(out):
                self.assertEqual(gate.main(["--spec", spec, "--command", "self-test", "hk"]), 0)
            answer = json.loads(out.getvalue())
            self.assertEqual((answer["host"], answer["nodes"]), ("hk", ["hk"]))
            self.assertEqual(answer["faults"], {"hk": ["docker-restart", "nft-drop-tcp443", "exit"]})

    def test_kill9_refuses_without_a_restart_policy(self):
        _, by_host = gates()
        with mock.patch.object(gate, "capture", return_value="4242 no"):
            with self.assertRaisesRegex(gate.Refused, "restart policy"):
                gate.daemon_host_pid(by_host["kr"], "kr")
        with mock.patch.object(gate, "capture", return_value="no"):
            with self.assertRaisesRegex(gate.Refused, "unit restart policy"):
                gate.daemon_host_pid(by_host["jp"], "jp")


class Render(unittest.TestCase):
    def test_authorized_keys_line_forces_the_gate_and_pins_the_collector(self):
        line = render.key_line("hk", "ssh-ed25519 AAAAC3 meshway-soak-collector\n")
        self.assertEqual(line, 'command="python3 ~/.meshway-soak-gate/gate.py --spec ~/.meshway-soak-gate/hk.json hk",'
                               'from="193.123.237.166",no-pty,no-port-forwarding,no-agent-forwarding,'
                               'no-X11-forwarding,no-user-rc ssh-ed25519 AAAAC3 meshway-soak-collector')
        config, _ = gates()
        text = render.keys(config, "ssh-ed25519 AAAAC3 c", "/b", "/g")
        self.assertIn("## kr: local to the collector", text)
        self.assertIn("scp /b/gate.py /g/planck.json u@planck.invalid:.meshway-soak-gate/", text)
        self.assertEqual(text.count('command="python3'), 4)

    def test_the_cron_block_replaces_only_its_own_lines(self):
        block = render.crontab("/home/ubuntu/meshway-soak")
        self.assertTrue(block.startswith(render.BEGIN) and block.rstrip().endswith(render.END))
        self.assertIn("*/5 * * * * flock -n $S/state/sample.lock python3 $S/bin/sample.py --config $S/config.json", block)
        self.assertIn("* * * * * flock -n $S/state/fault.lock python3 $S/bin/fault.py --config $S/config.json due", block)
        existing = "MAILTO=x\n0 1 * * * other job\n"
        once = render.replace_block(existing, block)
        self.assertEqual(render.replace_block(once, block), once)
        self.assertTrue(once.startswith(existing))
        self.assertEqual(render.replace_block(once, None), existing)
        self.assertEqual(render.replace_block("", None), "")


if __name__ == "__main__":
    unittest.main()
