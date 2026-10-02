#!/usr/bin/env python3
"""Controlled Meshway queue/worker/carrier A/B runs, retaining drop and kernel evidence."""

from datetime import datetime, timezone
import argparse
import json
import os
from pathlib import Path
import platform
import signal
import sys
import uuid

from benchmark import (LabError, architecture, benchmark_case, bounded_integer, docker,
                       docker_json, interrupted, now, positive_cpus, write_result)
from meshway import MESH_IP, RUN_DIRECTORY, MeshwayFixture, cpu_window

CONFIGURE = r"""
import json, os, sys
from pathlib import Path
path = Path('/run/meshway-lab/config.json')
config = json.loads(path.read_text())
config['dataplane']['limits']['queue_capacity'] = int(sys.argv[1])
if sys.argv[3] != 'default':
    config['dataplane']['limits']['work_budget_frames'] = int(sys.argv[3])
if sys.argv[4] != 'default':
    config['dataplane']['limits']['udp_receive_buffer_bytes'] = int(sys.argv[4])
if sys.argv[5] != 'default':
    config['dataplane']['limits']['native_tcp_nodelay'] = sys.argv[5] == 'on'
if sys.argv[2] == 'tcp':
    for peer in config['dataplane']['trusted_peers']:
        peer['transports'] = ['native_tcp' if t == 'native_udp' else t for t in peer['transports']]
    for group in ['listeners', 'outgoing']:
        for entry in config['dataplane'][group]:
            if entry['transport'] == 'native_udp':
                entry['transport'] = 'native_tcp'
temporary = path.with_suffix('.tmp')
fd = os.open(temporary, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
with os.fdopen(fd, 'w') as out:
    json.dump(config, out)
    out.flush()
    os.fsync(out.fileno())
temporary.replace(path)
"""

SAMPLE = r"""
import json, time
from pathlib import Path
root = Path('/run/meshway-lab')
record = json.loads((root / 'agent-pid.json').read_text())
process = Path('/proc') / str(record['pid'])
result = {'monotonic_ns': time.monotonic_ns(), 'pid': record['pid'], 'files': {}, 'tasks': {}}
for name in ['snmp', 'netstat', 'udp', 'udp6', 'softnet_stat']:
    path = Path('/proc/net') / name
    if path.exists(): result['files'][name] = path.read_text()
for path in process.joinpath('task').iterdir():
    try:
        status = dict(line.split(':', 1) for line in (path / 'status').read_text().splitlines() if ':' in line)
        result['tasks'][path.name] = {k: status[k].strip() for k in ['Name', 'voluntary_ctxt_switches', 'nonvoluntary_ctxt_switches'] if k in status}
    except FileNotFoundError: pass
result['thread_count'] = len(result['tasks'])
print(json.dumps(result))
"""


class DiagnosticFixture(MeshwayFixture):
    def generate_configs(self, second_leg):
        super().generate_configs(second_leg)
        for node in self.nodes:
            # Keys never leave their container or enter argv/results.
            budget = "default" if self.args.work_budget is None else str(self.args.work_budget)
            receive_buffer = "default" if self.args.udp_receive_buffer is None else str(self.args.udp_receive_buffer)
            response = self.exec(node, "python3", "-c", CONFIGURE, str(self.args.queue), self.args.first_leg, budget, receive_buffer, self.args.native_tcp_nodelay, check=False)
            if response.returncode:
                raise LabError(f"Diagnostic configuration failed for {node}; private output suppressed")

    def start_agent(self, node):
        self.agents_started.add(node)
        environment = [] if self.args.workers == 0 else ["--env", f"TOKIO_WORKER_THREADS={self.args.workers}"]
        docker("exec", "--detach", *environment, self.nodes[node], "python3", f"{RUN_DIRECTORY}/meshway_node.py", "supervise")

    def kernel(self):
        return {node: {
            "proc": json.loads(self.exec(node, "python3", "-c", SAMPLE).stdout),
            "udp_memory": self.exec(node, "ss", "-u", "-a", "-n", "-m").stdout,
            "links": json.loads(self.exec(node, "ip", "-json", "-s", "link", "show").stdout),
        } for node in self.nodes}


def counter_delta(before, after):
    result = {}
    for key, value in after.items():
        old = before.get(key)
        if isinstance(value, dict) and isinstance(old, dict):
            result[key] = counter_delta(old, value)
        elif type(value) in (int, float) and type(old) in (int, float):
            result[key] = value - old
    return result


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", default="meshway-agent-lab:local")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--queue", type=bounded_integer(1, 256), default=64)
    parser.add_argument("--workers", type=bounded_integer(0, 8), default=0, help="0: Tokio default; otherwise explicitly size its worker pool")
    parser.add_argument("--work-budget", type=bounded_integer(0, 64), default=None, help="explicit frames per cooperative yield; 0 disables the experimental yield budget")
    parser.add_argument("--udp-receive-buffer", type=bounded_integer(0, 4 * 1024 * 1024), default=None, help="requested UDP SO_RCVBUF; 0 preserves OS default, actual capacity is recorded by ss")
    parser.add_argument("--first-leg", choices=("udp", "tcp"), default="udp")
    parser.add_argument("--native-tcp-nodelay", choices=("default", "off", "on"), default="default", help="same-image Nagle A/B on both native TCP endpoints; VLESS remains NODELAY")
    parser.add_argument("--second-leg", choices=("udp", "vless"), default="udp", help="with first-leg=tcp, native UDP legs are both converted to TCP")
    parser.add_argument("--seconds", type=bounded_integer(1, 30), default=5)
    parser.add_argument("--omit", type=bounded_integer(0, 5), default=1)
    parser.add_argument("--iterations", type=bounded_integer(1, 5), default=1)
    parser.add_argument("--parallel", type=bounded_integer(1, 16), default=1)
    parser.add_argument("--reverse", action="store_true")
    parser.add_argument("--cpus", type=positive_cpus, default=1.0)
    parser.add_argument("--memory", choices=("128m", "256m", "512m", "1g"), default="256m")
    parser.add_argument("--temp-dir", type=Path)
    parser.add_argument("--ready-timeout", type=bounded_integer(5, 120), default=30)
    parser.add_argument("--skip-fault-check", action="store_true", help="for intermediate profiling only; final acceptance should retain the relay fault check")
    args = parser.parse_args(argv)
    # Required shared fixture settings; this diagnostic only runs TCP workloads.
    args.udp_rate, args.udp_length = "20M", 1200
    return args


def main(argv=None):
    args = arguments(argv)
    os.umask(0o077)
    output = args.output.expanduser().resolve()
    if output.exists() or not output.parent.is_dir():
        print("Output must be a new file in an existing directory", file=sys.stderr)
        return 2
    signal.signal(signal.SIGTERM, interrupted)
    result = {
        "schema_version": 1, "run_id": f"meshway-diag-{datetime.now(timezone.utc).strftime('%Y%m%d%H%M%S')}-{uuid.uuid4().hex[:8]}",
        "purpose": "Meshway controlled performance diagnosis; same E2E/auth/ordered path",
        "status": "running", "started_at": now(),
        "settings": {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
        "cases": [], "cleanup_errors": [],
    }
    fixture = DiagnosticFixture(args, result)
    code = 1
    try:
        info = docker_json("info", "--format", "{{json .}}")
        native = architecture(info["Architecture"])
        if info["OSType"] != "linux" or native != architecture(platform.machine()):
            raise LabError("Diagnostic requires native local Linux Docker")
        result["environment"] = {"host": list(platform.uname()), "daemon": {key: info.get(key) for key in ("ServerVersion", "OperatingSystem", "Architecture", "KernelVersion", "NCPU")}}
        fixture.prepare_image(native)
        write_result(output, result)
        result["readiness"] = fixture.setup_mesh(args.second_leg)
        result["fixture"] = fixture.snapshot()
        for iteration in range(1, args.iterations + 1):
            kernel_before, status_before, cpu_before = fixture.kernel(), fixture.statuses(), fixture.cgroups()
            case = benchmark_case(fixture, MESH_IP["NYC"], MESH_IP["SHA"], "tcp", args.parallel, args.reverse, iteration)
            case["cpu"] = cpu_window(cpu_before, fixture.cgroups())
            case["kernel_before"], case["kernel_after"] = kernel_before, fixture.kernel()
            case["agent_before"], case["agent_after"] = status_before, fixture.statuses()
            case["agent_counter_delta"] = counter_delta(status_before, case["agent_after"])
            result["cases"].append(case)
            write_result(output, result)
            rate = case["summary"]["receiver_bits_per_second"]
            display = f"{rate / 1e6:.2f} Mbit/s" if isinstance(rate, (int, float)) else "unavailable"
            print(f"{case['id']}: {display}, retransmits={case['summary']['retransmits']}", flush=True)
            if case["status"] != "ok": raise LabError("iperf3 diagnostic failed")
        if not args.skip_fault_check:
            result["relay_fault_check"] = fixture.fault_check()
        result["status"], code = "ok", 0
    except (LabError, OSError, ValueError, KeyError) as error:
        result["status"], result["error"] = "failed", str(error)
        print(str(error), file=sys.stderr)
    except KeyboardInterrupt:
        result["status"], code = "interrupted", 130
    finally:
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        result["cleanup_errors"] = fixture.cleanup()
        if result["cleanup_errors"]: result["status"], code = "cleanup_failed", 1
        result["finished_at"] = now()
        write_result(output, result)
        print(f"Results: {output} ({result['status']})", flush=True)
    return code


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
    from performance_lock import performance_lock
    with performance_lock():
        sys.exit(main())
