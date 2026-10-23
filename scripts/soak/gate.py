#!/usr/bin/env python3
"""Forced-command gate for the soak collector key, installed on every sampled host.

authorized_keys:
  command="python3 ~/.meshway-soak-gate/gate.py --spec ~/.meshway-soak-gate/<host>.json <host>",from="193.123.237.166",
  no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding,no-user-rc ssh-ed25519 AAAA… meshway-soak-collector

The requested command (`SSH_ORIGINAL_COMMAND`, or `--command` when the collector runs the gate on its own host) is
split with shlex and never given to a shell. It must match one of the forms below exactly, with every argument
checked against this host's spec (its nodes, containers, units, ping targets, iperf peers and the faults the
schedule names for it). Anything else is refused (exit 126) and logged to syslog and stderr. Allowed commands run as
fixed argument arrays (execvp); the only shell scripts are the fixed read-only probes defined in this file.

Read forms (every host, for its own nodes):
  status <node> · resources <node> <seconds 1-120> · inspect <node|server>
  journal <node> --after-cursor <cursor> | journal <node> --since <unix seconds>
  docker-logs <node|server> <since s.ns> <until s.ns> · docker-logs-count <node|server> <since s.ns> <until s.ns>
  ping <node> <target> · iperf-server <node> <port> <seconds> · iperf-client <node> <target> <port> <seconds> <mbit>
  self-test
Fault forms (only those the schedule names for this host):
  restart <node> · kill9 <node> · docker-restart <node>
  server-stop · server-start · server-rotate-key · server-upgrade <vX.Y.Z[-pre.N]>
  nft-drop <node> udp|tcp443 <seconds> · nft-clear <node>
  exit-select <node> <profile> · exit-default <node> · curl-loop <node> <seconds>

Python 3.8+ standard library, standalone (no import from the collector).
"""
from __future__ import annotations

import json
import os
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path

REFUSED = 126
TABLE = "meshway_soak_fault"
NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,62}$")
UNIT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.@-]{0,62}$")
CURSOR = re.compile(r"^s=[0-9a-f]{32};i=[0-9a-f]{1,16};b=[0-9a-f]{32};m=[0-9a-f]{1,16};t=[0-9a-f]{1,16};x=[0-9a-f]{1,16}$")
NANOS = re.compile(r"^[0-9]{10}\.[0-9]{9}$")
SECONDS = re.compile(r"^[0-9]{10}$")
VERSION = re.compile(r"^v[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}(-[a-z]{1,10}\.[0-9]{1,4})?$")
INT = re.compile(r"^[0-9]{1,6}$")

PROC = ("p=$({pid}); [ \"$p\" = 0 ] && p=; "
        "if [ -n \"$p\" ]; then st=$(cut -d' ' -f22 /proc/$p/stat); else p=none st=0; fi; "
        "echo \"@@proc $p $st $(awk '/^btime/{{print $2}}' /proc/stat) $(getconf CLK_TCK)\"")
RESOURCES = r"""
W={window}
p=$({pid}); [ -n "$p" ] && [ "$p" != 0 ] || {{ echo "@@error no meshwayd"; exit 0; }}
echo "@@pid $p $(cut -d' ' -f22 /proc/$p/stat) $(awk '/^btime/{{print $2}}' /proc/stat) $(getconf CLK_TCK) $(nproc)"
snap() {{
  echo "@@$1 $(date +%s.%N) $(awk '{{print $16+$17}}' /proc/$p/stat)"
  for t in /proc/$p/task/*; do echo "T ${{t##*/}} $(cut -d' ' -f1 $t/schedstat 2>/dev/null || echo 0) $(tr ' ' _ < $t/comm 2>/dev/null)"; done
  awk '/^(Udp|Tcp):/ {{print "N", $0}}' /proc/net/snmp
}}
snap a
sleep $W
snap b
echo "@@mem $(awk '/^VmRSS:/{{r=$2}} /^VmHWM:/{{h=$2}} END{{print r, h}}' /proc/$p/status)"
echo "@@fd $(ls /proc/$p/fd | wc -l) $(ls -l /proc/$p/fd 2>/dev/null | grep -c 'socket:')"
echo "@@exe $(sha256sum /proc/$p/exe | cut -c1-16)"
echo "@@load $(cut -d' ' -f1-3 /proc/loadavg)"
"""
CURL_LOOP = ("end=$(( $(date +%s) + {seconds} )); ok=0; n=0; "
             "while [ $(date +%s) -lt $end ]; do n=$((n+1)); "
             "curl -fsS -m 10 --max-filesize 65536 --limit-rate 64k -o /dev/null '{url}' && ok=$((ok+1)); sleep 2; done; "
             "echo \"@@curl $ok $n\"")
FAULT_FORMS = {"restart", "kill9", "docker-restart", "server-stop", "server-start", "server-rotate-key",
               "server-upgrade", "nft-drop", "nft-clear", "exit-select", "exit-default", "curl-loop"}


class Refused(Exception):
    pass


class Step:
    """One command: argv run without a shell; `stdin` text; `merge` stderr into stdout; `always` runs even after a failure."""

    def __init__(self, argv, stdin=None, merge=False, always=False, timeout=None):
        self.argv, self.stdin, self.merge, self.always, self.timeout = list(argv), stdin, merge, always, timeout

    def __repr__(self):
        return f"Step({self.argv!r}{', stdin' if self.stdin else ''}{', always' if self.always else ''})"


class Call:
    """A gate-side action that needs a runtime lookup (the daemon's host PID)."""

    def __init__(self, name, *args):
        self.name, self.args = name, args
        self.always = False

    def __repr__(self):
        return f"Call({self.name!r}, {self.args!r})"


# ---------------------------------------------------------------- planning (pure: the tests read it)

def need(condition, why):
    if not condition:
        raise Refused(why)


def number(text, low, high, what):
    need(INT.match(text) is not None, f"{what} is not a number")
    value = int(text)
    need(low <= value <= high, f"{what} {value} outside {low}-{high}")
    return value


class Gate:
    def __init__(self, spec: dict):
        self.spec = spec
        self.sudo = list(spec.get("sudo", ["sudo", "-n"]))
        self.docker = list(spec.get("docker", ["docker"]))
        self.nodes = spec.get("nodes", {})
        self.server = spec.get("server")
        self.faults = spec.get("faults", {})

    # -- spec lookups
    def node(self, name):
        need(NAME.match(name) is not None and name in self.nodes, f"node {name!r} is not on this host")
        spec = self.nodes[name]
        if spec.get("container"):
            need(NAME.match(spec["container"]) is not None, "bad container name in spec")
        else:
            need(UNIT.match(spec.get("unit", "meshwayd")) is not None, "bad unit name in spec")
        return spec

    def fault(self, target, form):
        need(form in self.faults.get(target, []), f"{form} {target} is not in this host's fault schedule")

    def in_node(self, spec, argv, interactive=False):
        """argv as root next to the node's meshwayd."""
        if spec.get("container"):
            return [*self.docker, "exec", *(["-i"] if interactive else []), spec["container"], *argv]
        return [*self.sudo, *argv]

    def pid_expr(self, spec):
        if spec.get("container"):
            return "pidof -s meshwayd"
        return f"systemctl show -p MainPID --value {spec.get('unit', 'meshwayd')}"

    def container_of(self, target):
        if target == "server":
            need(self.server is not None, "no Server on this host")
            return self.server["container"]
        spec = self.node(target)
        need(bool(spec.get("container")), f"{target} is not a container")
        return spec["container"]

    # -- the allow table
    def plan(self, command: str) -> list:
        need("\n" not in command and "\r" not in command and "\0" not in command, "control characters")
        try:
            words = shlex.split(command)
        except ValueError as error:
            raise Refused(f"unparsable: {error}")
        need(len(words) >= 1, "empty command")
        verb, args = words[0], words[1:]
        method = getattr(self, "form_" + verb.replace("-", "_"), None)
        need(method is not None, f"unknown form {verb!r}")
        return method(*args) if self._arity(method, args) else []

    @staticmethod
    def _arity(method, args):
        import inspect
        parameters = inspect.signature(method).parameters.values()
        required = sum(1 for p in parameters if p.default is p.empty)
        need(required <= len(args) <= len(parameters), f"{method.__name__[5:]} takes {required} arguments")
        return True

    # read forms
    def form_self_test(self):
        return [Call("self-test")]

    def form_status(self, node):
        spec = self.node(node)
        script = PROC.format(pid=self.pid_expr(spec)) + "; meshway --json status"
        return [Step(self.in_node(spec, ["sh", "-c", script]))]

    def form_resources(self, node, window):
        spec = self.node(node)
        seconds = number(window, 1, 120, "window")
        return [Step(self.in_node(spec, ["sh", "-c", RESOURCES.format(window=seconds, pid=self.pid_expr(spec))]))]

    def form_inspect(self, target):
        if target != "server":
            spec = self.node(target)
            if not spec.get("container"):
                unit = spec.get("unit", "meshwayd")
                return [Step(["systemctl", "show", "-p", "MainPID,Restart,NRestarts,ExecMainStartTimestamp", unit])]
        container = self.container_of(target)
        return [Step([*self.docker, "inspect", "-f",
                      "{{json .State}} {{.RestartCount}} {{.HostConfig.RestartPolicy.Name}} {{.Config.Image}}",
                      container])]

    def form_journal(self, node, how, value):
        spec = self.node(node)
        need(not spec.get("container"), f"{node} logs to docker, not journald")
        argv = [*self.sudo, "journalctl", "-u", spec.get("unit", "meshwayd"), "-o", "json", "--no-pager"]
        if how == "--after-cursor":
            need(CURSOR.match(value) is not None, "bad cursor")
            return [Step(argv + ["--after-cursor", value])]
        need(how == "--since" and SECONDS.match(value) is not None, "journal takes --after-cursor C or --since SECONDS")
        return [Step(argv + ["--since", "@" + value])]

    def form_docker_logs(self, target, since, until):
        container = self.container_of(target)
        need(NANOS.match(since) is not None and NANOS.match(until) is not None, "times are <s>.<9 digits>")
        need(since <= until, "since after until")
        return [Step([*self.docker, "logs", "--timestamps", "--since", since, "--until", until, container], merge=True)]

    def form_docker_logs_count(self, target, since, until):
        self.form_docker_logs(target, since, until)  # same checks
        return [Call("count", [*self.docker, "logs", "--timestamps", "--since", since, "--until", until,
                               self.container_of(target)])]

    def form_ping(self, node, target):
        spec = self.node(node)
        need(target in spec.get("ping", []), f"{node} may not ping {target!r}")
        return [Step(self.in_node(spec, ["meshway", "--json", "ping", "-c", "10", target]))]

    def form_iperf_server(self, node, port, seconds):
        spec = self.node(node)
        allowed = spec.get("iperf_server", [])
        need(number(port, 1024, 65535, "port") in allowed, f"{node} has no iperf server on {port}")
        limit = number(seconds, 5, 120, "seconds")
        return [Step(self.in_node(spec, ["timeout", str(limit), "iperf3", "-s", "-1", "-p", port]))]

    def form_iperf_client(self, node, target, port, seconds, mbit):
        spec = self.node(node)
        client = spec.get("iperf_client", {})
        need(target in client.get("targets", []) and number(port, 1024, 65535, "port") in client.get("ports", []),
             f"{node} may not iperf {target}:{port}")
        number(seconds, 1, 30, "seconds")
        number(mbit, 1, 100, "mbit")
        return [Step(self.in_node(spec, ["iperf3", "-c", target, "-p", port, "-t", seconds, "-b", mbit + "M", "-J"]))]

    # fault forms
    def form_restart(self, node):
        spec = self.node(node)
        self.fault(node, "restart")
        need(not spec.get("container"), "restart is for systemd nodes")
        return [Step([*self.sudo, "systemctl", "restart", spec.get("unit", "meshwayd")], timeout=120)]

    def form_kill9(self, node):
        spec = self.node(node)
        self.fault(node, "kill9")
        return [Call("kill9", node)]

    def form_docker_restart(self, node):
        container = self.container_of(node)
        self.fault(node, "docker-restart")
        return [Step([*self.docker, "restart", "-t", "30", container], timeout=120)]

    def form_server_stop(self):
        self.fault("server", "server-stop")
        return [Step([*self.docker, "stop", "-t", "30", self.container_of("server")], timeout=120)]

    def form_server_start(self):
        need("server-stop" in self.faults.get("server", []) or "server-start" in self.faults.get("server", []),
             "server-start is not in this host's fault schedule")
        return [Step([*self.docker, "start", self.container_of("server")], timeout=120)]

    def form_server_rotate_key(self):
        self.fault("server", "server-rotate-key")
        server = self.server
        container = self.container_of("server")
        window = ["--window", server["rotate_window"]] if server.get("rotate_window") else []
        if window:
            need(re.match(r"^[0-9]{1,3}h$", window[1]) is not None, "bad rotate window in spec")
        return [
            Step([*self.docker, "stop", "-t", "30", container], timeout=120),
            Step([*self.docker, "run", "--rm", "--network", "none", "--user", server["user"], "--cap-drop", "ALL",
                  "--security-opt", "no-new-privileges:true", "--read-only",
                  "-v", f"{server['release']}:/opt/meshway-server:ro", "-v", f"{server['data']}:/var/lib/meshway-server",
                  "--entrypoint", "/opt/meshway-server/meshway-server", server["image"],
                  "rotate-netmap-key", "--data-dir", "/var/lib/meshway-server", *window], merge=True, timeout=300),
            Step([*self.docker, "start", container], always=True, timeout=120),
        ]

    def form_server_upgrade(self, version):
        self.fault("server", "server-upgrade")
        need(VERSION.match(version) is not None, "bad version")
        server = self.server
        stage = Path(server["stage"]) / version
        base = f"{server['release_url']}/{version}"
        steps = [Step(["mkdir", "-p", str(stage)])]
        for name in (server["asset"], "SHA256SUMS"):
            if not (stage / name).exists():
                steps.append(Step(["curl", "-fsSL", "--proto", "=https", "-o", str(stage / name), f"{base}/{name}"],
                                  timeout=600))
        steps.append(Step([*self.sudo, "sh", server["upgrade_script"], str(stage / server["asset"]),
                           str(stage / "SHA256SUMS")], merge=True, timeout=900))
        return steps

    def form_nft_drop(self, node, match, seconds):
        container = self.container_of(node)
        self.fault(node, f"nft-drop-{match}")
        limit = number(seconds, 10, 1800, "seconds")
        if match == "udp":
            chains = "".join(" chain %s { type filter hook %s priority -150; policy accept; %smeta l4proto udp drop; }\n"
                             % (hook, hook, lo) for hook, lo in (("input", "iif lo accept; "),
                                                                 ("output", "oif lo accept; "), ("forward", "")))
        else:
            need(match == "tcp443", "match is udp or tcp443")
            chains = " chain output { type filter hook output priority -150; policy accept; tcp dport 443 drop; }\n"
        rules = "table inet %s {\n%s}\n" % (TABLE, chains)
        return [
            # The dead man removes the table even if the collector or its connection dies.
            Step([*self.docker, "exec", "-d", container, "sh", "-c",
                  f"sleep {limit + 120}; nft delete table inet {TABLE}"], timeout=30),
            Step([*self.docker, "exec", "-i", container, "nft", "-f", "-"], stdin=rules, timeout=30),
            Step([*self.docker, "exec", container, "nft", "list", "table", "inet", TABLE], timeout=30),
        ]

    def form_nft_clear(self, node):
        container = self.container_of(node)
        need(any(f.startswith("nft-drop-") for f in self.faults.get(node, [])), "no nft fault for this node")
        return [Step([*self.docker, "exec", container, "nft", "delete", "table", "inet", TABLE], merge=True, timeout=30)]

    def exit_spec(self, node):
        spec = self.node(node)
        self.fault(node, "exit")
        return spec, spec.get("exit", {})

    def form_exit_select(self, node, profile):
        spec, exit_ = self.exit_spec(node)
        need(profile in exit_.get("profiles", []), f"profile {profile!r} not allowed")
        return [Step(self.in_node(spec, ["meshway", "--json", "exit", profile]), timeout=90)]

    def form_exit_default(self, node):
        spec, _ = self.exit_spec(node)
        return [Step(self.in_node(spec, ["meshway", "--json", "exit", "--default"]), timeout=90)]

    def form_curl_loop(self, node, seconds):
        spec, exit_ = self.exit_spec(node)
        limit = number(seconds, 10, 900, "seconds")
        url = exit_.get("url", "")
        need(re.match(r"^https://[A-Za-z0-9.-]+(/[A-Za-z0-9._/-]*)?$", url) is not None, "bad curl URL in spec")
        return [Step(self.in_node(spec, ["sh", "-c", CURL_LOOP.format(seconds=limit, url=url)]), timeout=limit + 60)]


# ---------------------------------------------------------------- execution

def log(level: int, message: str) -> None:
    try:
        import syslog
        syslog.openlog("meshway-soak-gate", 0, syslog.LOG_AUTH)
        syslog.syslog(level, message)
    except Exception:
        pass


def capture(argv, timeout=30) -> str:
    return subprocess.run(argv, capture_output=True, text=True, timeout=timeout).stdout.strip()


def daemon_host_pid(gate: Gate, node: str) -> int:
    spec = gate.nodes[node]
    if not spec.get("container"):
        policy = capture(["systemctl", "show", "-p", "Restart", "--value", spec.get("unit", "meshwayd")])
        if policy not in ("on-failure", "always", "on-abnormal"):
            raise Refused(f"unit restart policy is {policy!r}; kill -9 would leave it down")
        return int(capture(["systemctl", "show", "-p", "MainPID", "--value", spec.get("unit", "meshwayd")]) or 0)
    state = capture([*gate.docker, "inspect", "-f", "{{.State.Pid}} {{.HostConfig.RestartPolicy.Name}}",
                     spec["container"]]).split()
    if len(state) != 2 or state[1] not in ("unless-stopped", "always", "on-failure"):
        raise Refused(f"container restart policy is {state[1:] or '?'}; kill -9 would leave it down")
    init = state[0]
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
            comm = (entry / "comm").read_text().strip()
        except OSError:
            continue
        if fields[1] == init and comm == "meshwayd":
            return int(entry.name)
    return 0


def run_call(gate: Gate, call: Call) -> int:
    if call.name == "self-test":
        print(json.dumps({"gate": "meshway-soak", "host": gate.spec.get("host"), "nodes": sorted(gate.nodes),
                          "server": bool(gate.server), "faults": gate.faults,
                          "sudo": subprocess.run([*gate.sudo, "true"], capture_output=True).returncode == 0}))
        return 0
    if call.name == "count":  # lines in a window, counted here so they do not cross the connection again
        process = subprocess.Popen(call.args[0], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        count = sum(1 for _ in process.stdout)
        print(f"@@count {count}")
        return process.wait()
    if call.name == "kill9":
        pid = daemon_host_pid(gate, call.args[0])
        if pid <= 1:
            raise Refused("meshwayd is not running")
        print(f"@@killed {pid}")
        sys.stdout.flush()
        return subprocess.run([*gate.sudo, "kill", "-9", str(pid)]).returncode
    raise Refused(call.name)


def execute(gate: Gate, steps: list) -> int:
    if len(steps) == 1 and isinstance(steps[0], Step) and steps[0].stdin is None:
        step = steps[0]
        sys.stdout.flush()
        if step.merge:
            os.dup2(1, 2)
        os.execvp(step.argv[0], step.argv)  # no shell; replaces the gate
    rc = 0
    for step in steps:
        if rc != 0 and not step.always:
            continue
        if isinstance(step, Call):
            rc = run_call(gate, step)
            continue
        try:
            done = subprocess.run(step.argv, input=step.stdin, text=True, timeout=step.timeout,
                                  stderr=subprocess.STDOUT if step.merge else None)
            result = done.returncode
        except subprocess.TimeoutExpired:
            print(f"@@timeout {step.argv[0]} after {step.timeout} s", file=sys.stderr)
            result = 124
        if rc == 0:
            rc = result
    return rc


def main(argv: list) -> int:
    spec_path, command, rest = None, None, []
    iterator = iter(argv)
    for arg in iterator:
        if arg == "--spec":
            spec_path = next(iterator, None)
        elif arg == "--command":
            command = next(iterator, None)
        else:
            rest.append(arg)
    if command is None:
        command = os.environ.get("SSH_ORIGINAL_COMMAND", "")
    host = rest[0] if rest else "?"
    try:
        spec = json.loads(Path(os.path.expanduser(spec_path or "")).read_text())
        if spec.get("host") != host:
            raise Refused(f"spec is for {spec.get('host')!r}, not {host!r}")
        gate = Gate(spec)
        steps = gate.plan(command)
    except (Refused, OSError, ValueError, KeyError) as error:
        message = f"refused on {host}: {error}; command {command[:200]!r}"
        log(4, message)
        print(f"meshway-soak-gate: {message}", file=sys.stderr)
        return REFUSED
    log(6, f"{host}: {command[:200]}")
    try:
        return execute(gate, steps)
    except Refused as error:
        print(f"meshway-soak-gate: refused on {host}: {error}", file=sys.stderr)
        log(4, f"refused on {host}: {error}")
        return REFUSED


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
