#!/usr/bin/env python3
"""Isolated, resource-matched TCP/TUN throughput and loss investigation."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "experiments/transport"))
from benchmark import (Fixture, LabError, architecture, benchmark_case, bounded_integer, positive_cpus, docker,
                       docker_json, interrupted, now, wait_until, write_result)
from diagnose import DiagnosticFixture, counter_delta
from meshway import MESH_IP, RUN_DIRECTORY, TUN_NAME, authenticated_graph, cpu_window
from performance_lock import performance_lock


def environment(info):
    result = {'host':list(platform.uname()), 'host_logical_cpus':os.cpu_count(),
              'daemon':{k:info.get(k) for k in ['ServerVersion','OperatingSystem','Architecture','KernelVersion','NCPU','MemTotal']}}
    if platform.system() == 'Darwin':
        command = ['sysctl','-n','hw.physicalcpu','hw.logicalcpu','hw.memsize','machdep.cpu.brand_string']
        sample = subprocess.run(command,capture_output=True,text=True,check=False)
        result['host_hardware'] = {'command':command,'exit_code':sample.returncode,'stdout':sample.stdout,'stderr':sample.stderr}
    return result

DIRECT = r"""
import json, os, sys
from pathlib import Path
p = Path('/run/meshway-lab/config.json')
c = json.loads(p.read_text())
d = c['dataplane']
n = d['node_id']
peer = 'sha' if n == 'nyc' else 'nyc'
d['trusted_peers'] = [x for x in d['trusted_peers'] if x['node_id'] == peer]
d['trusted_peers'][0]['transports'] = ['native_udp']
d['allowed_paths'] = [['nyc', 'sha'], ['sha', 'nyc']]
d['listeners'] = [{'transport': 'native_udp', 'bind': '0.0.0.0:41000'}]
d['outgoing'] = [] if n == 'nyc' else [{'peer': 'nyc', 'transport': 'native_udp', 'endpoint': sys.argv[1], 'priority': 10}]
c['tun']['send_path'] = [n, peer]
c['tun']['receive_path'] = [peer, n]
t = p.with_suffix('.tmp')
fd = os.open(t, os.O_CREAT|os.O_EXCL|os.O_WRONLY, 0o600)
with os.fdopen(fd, 'w') as f:
    json.dump(c, f); f.flush(); os.fsync(f.fileno())
t.replace(p)
"""

PROCESS = r"""
import json, time
from pathlib import Path
r = {'monotonic_ns': time.monotonic_ns(), 'files': {}}
r['sysctls'] = {}
for name in ['net/ipv4/tcp_rmem','net/ipv4/tcp_wmem','net/ipv4/tcp_congestion_control','net/core/rmem_max','net/core/wmem_max']:
 p = Path('/proc/sys')/name
 if p.exists(): r['sysctls'][name] = p.read_text().strip()
pid = Path('/run/meshway-lab/agent-pid.json')
if pid.exists():
 p = Path('/proc')/str(json.loads(pid.read_text())['pid'])
 for n in ['stat', 'status', 'io', 'schedstat']:
  try: r['files'][n] = (p/n).read_text()
  except FileNotFoundError: pass
for n in ['snmp', 'netstat', 'softnet_stat']:
 r['files']['net/'+n] = (Path('/proc/net')/n).read_text()
print(json.dumps(r))
"""


class TunnelFixture(DiagnosticFixture):
    def setup_direct(self):
        self.create_network('a', 'tun-direct')
        self.create_node('NYC', 'a', 'tun-direct')
        self.create_node('SHA', 'a', 'tun-direct')
        self.remove_defaults()
        # Reuse the authoritative Rust lab generator; transform only topology.
        # Both keys stay private; the unused generated relay identity is discarded.
        self.networks['b'] = self.networks['a']
        self.nodes['LAX'] = self.nodes['NYC']
        try:
            # Parent copies three files; avoid duplicating NYC's private file.
            original = self.copy_file
            self.copy_file = lambda node, source, dest: None if node == 'LAX' else original(node, source, dest)
            self.generate_configs('udp')
        finally:
            self.copy_file = original
            del self.nodes['LAX']
            del self.networks['b']
        for node in self.nodes:
            response = self.exec(node, 'python3', '-c', DIRECT, self.address('NYC', 'a') + ':41000', check=False)
            if response.returncode:
                raise LabError('Direct private config transform failed; output suppressed')
            self.start_agent(node)
        def ready():
            s = self.statuses()
            return len(s) == 2 and all(s[peer]['node_id'] in s[node]['authenticated_peers'] for node, peer in [('NYC','SHA'),('SHA','NYC')])
        wait_until('authenticated direct peer graph', ready, timeout=self.args.ready_timeout)
        if getattr(self.args, 'runtime_only', False):
            return {'authenticated_direct': True, 'statuses': self.statuses()}
        for node, peer in [('NYC','SHA'),('SHA','NYC')]:
            wait_until('direct TUN up', lambda n=node: self.tun_ready(n))
            self.exec(node, 'ip', 'route', 'replace', MESH_IP[peer]+'/32', 'dev', TUN_NAME, 'src', MESH_IP[node])
            wait_until('direct TUN ping', lambda n=node,p=peer: self.ping(n,p)['exit_code'] == 0)
        return {'authenticated_direct': True, 'statuses': self.statuses(), 'connections': self.connections()}


def cpu_details(window, received_bytes):
    total = 0
    for node, entry in window['nodes'].items():
        stats = []
        for side in ['before','after']:
            stats.append(dict(line.split() for line in window[side][node]['files']['cpu.stat'].splitlines()))
        for field in ['user_usec','system_usec']:
            entry[field+'_delta'] = int(stats[1][field])-int(stats[0][field])
        total += entry.get('usage_usec_delta', 0)
    window['total_cpu_ns_per_receiver_payload_byte'] = total*1000/received_bytes if received_bytes else None
    window['alignment'] = 'Container windows include omitted warmup and Docker sampling overhead; ns/B is an upper estimate, not aligned in-process CPU.'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image', required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--temp-dir', type=Path, required=True)
    p.add_argument('--modes', nargs='+', choices=['bare','direct','relay'], default=['bare','direct','relay'])
    p.add_argument('--bare-mtus', type=int, nargs='+', choices=[1000,1280,1500], default=[1500,1280])
    p.add_argument('--seconds', type=bounded_integer(1,30), default=10)
    p.add_argument('--iterations', type=bounded_integer(1,3), default=1)
    p.add_argument('--queue', type=bounded_integer(1,256), default=64)
    p.add_argument('--work-budget', type=bounded_integer(0,64), default=16)
    p.add_argument('--udp-receive-buffer', type=bounded_integer(0,4*1024*1024), default=0)
    p.add_argument('--cpus', type=positive_cpus, default=1)
    p.add_argument('--workers', type=bounded_integer(1,8), default=2)
    args = p.parse_args()
    args.memory, args.mtu, args.omit = '256m', 1500, 2
    args.first_leg, args.native_tcp_nodelay, args.ready_timeout = 'udp', 'default', 30
    args.udp_rate, args.udp_length = '20M', 1200
    os.umask(0o077)
    if args.output.exists() or not args.output.parent.is_dir():
        raise ValueError('Output must be a new file with an existing parent')
    signal.signal(signal.SIGTERM, interrupted)
    result = {'schema_version':1, 'run_id': 'meshway-throughput-'+datetime.now(timezone.utc).strftime('%Y%m%d%H%M%S')+'-'+uuid.uuid4().hex[:8],
              'started_at':now(), 'status':'running', 'settings':{k:str(v) if isinstance(v,Path) else v for k,v in vars(args).items()},
              'scope':'Static transport-lab, real Runtime/TUN/E2E/hop Noise/path auth. Not controller-managed pipeline; no TLS/VLESS in this matrix.',
              'cases':[], 'fixtures':{}, 'cleanup_errors':[]}
    info = docker_json('info','--format','{{json .}}')
    native = architecture(info['Architecture'])
    if info['OSType'] != 'linux' or native != architecture(platform.machine()):
        raise LabError('Native local Linux Docker required')
    result['environment'] = environment(info)
    fixture = None
    try:
        for mode in args.modes:
            fixture = Fixture(args,result) if mode == 'bare' else TunnelFixture(args,result)
            fixture.prepare_image(native)
            if mode == 'bare':
                source,target,proof = fixture.setup('direct')
                # Bare default (offloads retained) and exact tunnel MTU below.
            else:
                proof = fixture.setup_direct() if mode == 'direct' else fixture.setup_mesh('udp')
                source,target = MESH_IP['NYC'],MESH_IP['SHA']
            result['fixtures'][mode] = {'snapshot':fixture.snapshot(), 'readiness':proof,
                'offload':{n:fixture.exec(n,'ethtool','-k','eth0',check=False).stdout for n in fixture.nodes}}
            for mtu in (args.bare_mtus if mode == 'bare' else [1280]):
                if mode == 'bare':
                    for n in fixture.nodes: fixture.exec(n,'ip','link','set','eth0','mtu',str(mtu))
                for iteration in range(1,args.iterations+1):
                    for parallel in [1,4]:
                        for reverse in [False,True]:
                            before = {n:json.loads(fixture.helper(n,'cgroup').stdout) for n in fixture.nodes}
                            processes = {n:json.loads(fixture.exec(n,'python3','-c',PROCESS).stdout) for n in fixture.nodes}
                            status = fixture.statuses() if mode != 'bare' else {}
                            kernel = fixture.kernel() if mode != 'bare' else {}
                            # iperf --logfile appends; each same-name case must start empty.
                            log_name = f"/run/tcp-p{parallel}-{'reverse' if reverse else 'forward'}-{iteration}.json"
                            fixture.exec('SHA','rm','-f',log_name)
                            case = benchmark_case(fixture,source,target,'tcp',parallel,reverse,iteration)
                            after = {n:json.loads(fixture.helper(n,'cgroup').stdout) for n in fixture.nodes}
                            status_after = fixture.statuses() if mode != 'bare' else {}
                            case.update(mode=mode,mtu=mtu,hop_count=2 if mode=='relay' else 1,process_before=processes,
                                process_after={n:json.loads(fixture.exec(n,'python3','-c',PROCESS).stdout) for n in fixture.nodes},
                                kernel_before=kernel,kernel_after=fixture.kernel() if mode!='bare' else {},
                                agent_before=status,agent_after=status_after,
                                agent_counter_delta=counter_delta(status,status_after) if mode!='bare' else {})
                            case['cpu'] = cpu_window(before,after)
                            cpu_details(case['cpu'],case['raw_client'].get('end',{}).get('sum_received',{}).get('bytes',0))
                            result['cases'].append(case)
                            write_result(args.output,result)
                            print(f"{mode} mtu{mtu} {case['id']}: {case['summary']['receiver_bits_per_second']/1e6:.2f} Mbit/s retrans={case['summary']['retransmits']}",flush=True)
                            if case['status'] != 'ok': raise LabError('iperf case failed')
            if mode == 'relay': result['relay_fault_check'] = fixture.fault_check()
            errors = fixture.cleanup()
            result['cleanup_errors'].extend(errors)
            fixture = None
            if errors: raise LabError('Cleanup failed')
        result['status'] = 'ok'
    except KeyboardInterrupt:
        result['status'] = 'interrupted'
        raise
    except Exception as e:
        result['status'],result['error'] = 'failed',str(e)
        raise
    finally:
        if fixture: result['cleanup_errors'].extend(fixture.cleanup())
        result['finished_at'] = now()
        write_result(args.output,result)


if __name__ == '__main__':
    with performance_lock():
        main()
