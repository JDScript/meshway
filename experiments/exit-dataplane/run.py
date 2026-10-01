#!/usr/bin/env python3
"""Isolated C-[R]-E-S / C-{E,F}-S real Noise/TUN/NAT acceptance."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
RUN = '/run/meshway-exit'
sys.path.insert(0, str(ROOT / 'experiments/transport'))
from performance_lock import performance_environment, performance_lock, validate_descriptor

LOCK_DESCRIPTOR = None


def docker_process_options():
    if LOCK_DESCRIPTOR is None:
        raise RuntimeError('Docker operations require the shared Meshway performance lock')
    validate_descriptor(LOCK_DESCRIPTOR)
    return {'pass_fds': (LOCK_DESCRIPTOR,), 'env': {**performance_environment(), 'MESHWAY_PERF_LOCK_FD': str(LOCK_DESCRIPTOR)}}


def command(*args, check=True, timeout=60):
    p = subprocess.run(args, capture_output=True, text=True, timeout=timeout,
                       **(docker_process_options() if args[0] == 'docker' else {}))
    if check and p.returncode:
        raise RuntimeError(f'Command failed: {args[:4]}: {p.stderr[-1500:]}')
    return p


def docker(*args, **kwargs):
    return command('docker', *args, **kwargs)


def wait(label, predicate, timeout=30):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        if predicate():
            return
        time.sleep(.15)
    raise RuntimeError('Timeout: ' + label)


class Fixture:
    def __init__(self, args, result, case):
        self.args, self.result, self.case = args, result, case
        self.prefix = result['run_id'] + '-' + case
        self.nodes, self.nets, self.owned_nodes, self.owned_nets = {}, {}, [], []
        self.addresses = {}
        self.checks = []
        result['cases'].append({'case': case, 'checks': self.checks})

    def labels(self):
        return ['--label', 'meshway.exit.owner=' + self.result['run_id']]

    def network(self, key, subnet):
        name = self.prefix + '-' + key
        self.owned_nets.append(name)
        docker('network', 'create', *self.labels(), '--internal', '--subnet', subnet, name)
        self.nets[key] = name

    def node(self, node, net, address, forwarding=False):
        name = self.prefix + '-' + node.lower()
        self.owned_nodes.append(name)
        docker('run', '-d', '--name', name, *self.labels(), '--network', self.nets[net], '--ip', address,
               '--cap-drop', 'ALL', '--cap-add', 'NET_ADMIN', '--cap-add', 'NET_RAW', '--device', '/dev/net/tun',
               '--security-opt', 'no-new-privileges', '--read-only', '--tmpfs', '/run:rw,nosuid,nodev,mode=0700,size=32m',
               '--tmpfs', '/tmp:rw,nosuid,nodev,noexec,mode=0700,size=16m', '--cpus', '1', '--memory', '256m',
               '--memory-swap', '256m', '--pids-limit', '64', '--sysctl', f'net.ipv4.ip_forward={int(forwarding)}',
               '--sysctl', 'net.ipv6.conf.all.disable_ipv6=1', '--sysctl', 'net.ipv6.conf.default.disable_ipv6=1',
               '--entrypoint', 'python3', self.args.image, '-c', 'import signal; signal.pause()')
        self.nodes[node] = name
        self.addresses[node, net] = address
        self.exec(node, 'mkdir', '-m', '700', RUN)
        self.copy(node, HERE / 'node.py', RUN + '/node.py')

    def copy(self, node, source, destination):
        # Docker's archive API cannot address some tmpfs mounts (OrbStack).
        # Stream bytes into the actual namespace; do not print private fixtures.
        p = subprocess.run(['docker', 'exec', '-i', self.nodes[node], 'python3', '-c',
                            'import os,sys; f=os.open(sys.argv[1],os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600); '
                            's=os.fdopen(f,"wb"); s.write(sys.stdin.buffer.read()); s.close()', destination],
                           input=Path(source).read_bytes(), capture_output=True, timeout=30, **docker_process_options())
        if p.returncode:
            raise RuntimeError('Namespace byte transfer failed; private data suppressed')

    def connect(self, node, net, address):
        docker('network', 'connect', '--ip', address, self.nets[net], self.nodes[node])
        self.addresses[node, net] = address

    def exec(self, node, *args, **kwargs):
        return docker('exec', self.nodes[node], *args, **kwargs)

    def detached(self, node, *args):
        docker('exec', '-d', self.nodes[node], *args)

    def read(self, node, file):
        response = self.exec(node, 'cat', RUN + '/' + file, check=False)
        return json.loads(response.stdout) if response.returncode == 0 else None

    def touch(self, node, name):
        self.exec(node, 'touch', RUN + '/' + name)

    def check(self, name, success, **evidence):
        self.checks.append({'name': name, 'status': 'ok' if success else 'failed', **evidence})
        print(self.case + ': ' + name + (' ok' if success else ' FAILED'), flush=True)
        if not success:
            raise RuntimeError(self.case + ': ' + name)

    def http(self, node, target):
        p = self.exec(node, 'curl', '--noproxy', '*', '--silent', '--show-error', '--max-time', '2', f'http://{target}:8080/', check=False)
        return {'exit_code': p.returncode, 'stdout': p.stdout, 'stderr': p.stderr}

    def succeeds(self, target, expected_source):
        wait('authorized external HTTP', lambda: self.http('C', target)['exit_code'] == 0)
        response = self.http('C', target)
        self.check('external server observes NAT source ' + expected_source,
                   response['exit_code'] == 0 and json.loads(response['stdout'])['source'] == expected_source, response=response)

    def setup(self, mode='direct', default=False, authorized=True, lease=180, locked=False, direct_count=0, internet_only=False):
        self.network('a', '172.29.231.0/24')
        self.network('ext', '172.29.233.0/24')
        self.node('C', 'a', '172.29.231.10')
        if mode == 'relay':
            self.network('b', '172.29.232.0/24')
            self.node('R', 'a', '172.29.231.11')
            self.connect('R', 'b', '172.29.232.11')
            self.node('E', 'b', '172.29.232.12', forwarding=True)
            order, endpoints = ['C', 'R', 'E'], ['172.29.231.10:41170', '172.29.232.11:41170', '172.29.232.12:41170']
        else:
            self.node('E', 'a', '172.29.231.12', forwarding=True)
            order, endpoints = ['C', 'E'], ['172.29.231.10:41170', '172.29.231.12:41170']
        self.connect('E', 'ext', '172.29.233.10')
        if mode == 'dual':
            self.node('F', 'a', '172.29.231.13', forwarding=True)
            self.connect('F', 'ext', '172.29.233.11')
            order.append('F')
            endpoints.append('172.29.231.13:41170')
        self.node('S', 'ext', '172.29.233.20')
        self.exec('S', 'ip', 'address', 'add', '172.29.233.21/24', 'dev', 'eth0')
        self.node('L', 'a', '172.29.231.20')
        for node in self.nodes:
            # No gateway transit / Docker Internet. Only directly connected networks.
            self.exec(node, 'ip', '-4', 'route', 'del', 'default', check=False)
        for node in ['S', 'L']:
            self.detached(node, 'python3', RUN + '/node.py', 'server')
        self.detached('S', 'iperf3', '-s')
        wait('S helper', lambda: self.http('E', '172.29.233.20')['exit_code'] == 0)
        self.check('S only attached to external network',
                   len(json.loads(docker('inspect', self.nodes['S']).stdout)[0]['NetworkSettings']['Networks']) == 1)
        self.check('C has no underlay path to S', self.http('C', '172.29.233.20')['exit_code'] != 0)
        prefix = '0.0.0.0/0' if default else '172.29.233.20/32'
        with tempfile.TemporaryDirectory(dir=self.args.artifacts, prefix='private-exit-fixtures-') as d:
            generator = self.prefix + '-generator'
            self.owned_nodes.append(generator)
            response = docker('run', '--name', generator, *self.labels(), '--network', 'none', '--cap-drop', 'ALL',
                              '--read-only', '--security-opt', 'no-new-privileges', '--user', f'{os.getuid()}:{os.getgid()}',
                              '--mount', f'type=bind,source={d},target=/lab', '--env', 'MESHWAY_EXIT_DIRECTORY=/lab',
                              '--env', 'MESHWAY_EXIT_ENDPOINTS=' + ','.join(endpoints), '--env', 'MESHWAY_EXIT_PREFIX=' + prefix,
                              '--env', f'MESHWAY_EXIT_AUTHORIZED={int(authorized)}', '--env', f'MESHWAY_EXIT_DEFAULTS={int(mode == "dual")}',
                              '--env', f'MESHWAY_EXIT_LOCKED={int(locked)}', '--env', f'MESHWAY_EXIT_LEASE={lease}',
                              '--env', f'MESHWAY_EXIT_DIRECT_COUNT={direct_count}',
                              '--env', f'MESHWAY_EXIT_INTERNET_ONLY={int(internet_only)}',
                              self.args.image, '--exact', 'managed_dataplane::exit::fixture::generate', '--ignored', check=False)
            if response.returncode:
                raise RuntimeError('Private fixture generation failed; output suppressed')
            for i, node in enumerate(order):
                self.copy(node, Path(d) / f'node-{i}.json', RUN + '/config.json')
        for node in order:
            docker('exec', '-d', '--env', 'MESHWAY_EXIT_TARGET=172.29.233.20', self.nodes[node],
                   'python3', RUN + '/node.py', 'agent')
        for node in order:
            def ready(n=node):
                if self.read(n, 'agent-exit.json'):
                    error = self.exec(n, 'cat', RUN + '/agent.log', check=False).stdout
                    raise RuntimeError(f'{n} fixture exited: {error[-2500:]}')
                return bool(self.read(n, 'ready'))
            wait('Agent ' + node, ready)
        return order

    def flow(self, target='172.29.233.20', name='flow.json'):
        self.detached('C', 'python3', RUN + '/node.py', 'flow', target, name)
        wait('live TCP stream', lambda: (self.read('C', name) or {}).get('successes', 0) >= 5 and self.read('C', name).get('status') == 'active')
        return name

    def flow_stopped(self, name):
        wait('existing TCP stream terminated', lambda: (self.read('C', name) or {}).get('status') == 'terminated', timeout=8)
        self.check('existing TCP flow is data-plane terminated', True, stream=self.read('C', name))

    def kill(self, node):
        # Child identity is namespace-local, under an owning live supervisor.
        identity = self.read(node, 'agent-pid.json')
        self.exec(node, 'python3', '-c',
                  "import os,signal,sys; from pathlib import Path; pid=int(sys.argv[1]); "
                  "ticks=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[19]; "
                  "assert ticks==sys.argv[2], 'Agent PID reused'; os.kill(pid,signal.SIGKILL)",
                  str(identity['pid']), identity['start_ticks'])
        wait('killed Agent supervisor', lambda: bool(self.read(node, 'agent-exit.json')))

    def selection(self, value, accept=True):
        previous = self.read('C', 'ready')
        self.exec('C', 'rm', '-f', RUN + '/selection-result')
        self.exec('C', 'python3', '-c', 'from pathlib import Path; import sys; Path(sys.argv[1]).write_text(sys.argv[2])',
                  RUN + '/selection-request', value)
        wait('selection response', lambda: self.exec('C', 'test', '-f', RUN + '/selection-result', check=False).returncode == 0)
        if accept:
            wait('new selected generation', lambda: self.read('C', 'ready')['generation'] > previous['generation'])
        text = self.exec('C', 'cat', RUN + '/selection-result').stdout
        self.check('selection ' + value[-8:] + (' accepted' if accept else ' rejected'),
                   text == 'accepted' if accept else text.startswith('rejected:'), result=text,
                   before=previous, after=self.read('C', 'ready'))
        self.check('selection does not change policy revision', previous['revision'] == self.read('C', 'ready')['revision'])

    def baseline(self):
        for node in ['E', 'C']:
            ping = self.exec(node, 'ping', '-n', '-c', '5', '-W', '1', '172.29.233.20', check=False)
            iperf = self.exec(node, 'iperf3', '-c', '172.29.233.20', '-t', '3', '-P', '1', '-J', check=False)
            report = json.loads(iperf.stdout)
            self.check('short baseline ' + node, ping.returncode == 0 and iperf.returncode == 0 and 'error' not in report,
                       rtt=ping.stdout, throughput=report, settings={'cpus': 1, 'workers': 2, 'tun_mtu': 1000, 'parallel': 1, 'seconds': 3})

    def cleanup(self):
        errors = []
        for kind, names in [('container', self.owned_nodes), ('network', self.owned_nets)]:
            for name in reversed(names):
                inspected = docker(kind, 'inspect', name, check=False)
                if inspected.returncode:
                    continue
                resource = json.loads(inspected.stdout)[0]
                labels = resource['Config']['Labels'] if kind == 'container' else resource['Labels']
                if labels.get('meshway.exit.owner') != self.result['run_id']:
                    errors.append('ownership mismatch ' + name)
                    continue
                removed = docker(kind, 'rm', *(['-f'] if kind == 'container' else []), name, check=False)
                if removed.returncode:
                    errors.append(name + ': ' + removed.stderr)
        return errors


def exercise(f, case):
    if case in ['direct', 'relay']:
        f.setup(mode=case)
        f.succeeds('172.29.233.20', '172.29.233.10')
        local = f.http('C', '172.29.231.20')
        route = f.exec('C', 'ip', '-j', 'route', 'get', '172.29.231.20').stdout
        f.check('unmatched prefix remains local', local['exit_code'] == 0 and json.loads(local['stdout'])['source'] == '172.29.231.10', response=local, route=json.loads(route))
        f.baseline()
        if case == 'relay':
            f.kill('R')
            f.check('mandatory relay cannot be bypassed', f.http('C', '172.29.233.20')['exit_code'] != 0)
        else:
            before = f.read('S', 'udp.json')['received']
            f.touch('C', 'inject-forged')
            wait('authenticated forged packet sent', lambda: f.exec('C', 'test', '-f', RUN + '/forged-sent', check=False).returncode == 0)
            time.sleep(.5)
            f.check('authenticated forged inner source rejected', f.read('S', 'udp.json')['received'] == before)
            f.detached('C', 'python3', RUN + '/node.py', 'server')
            wait('C unsolicited ingress sink', lambda: f.read('C', 'udp.json') is not None)
            f.touch('E', 'inject-unsolicited')
            wait('authenticated unsolicited packet sent', lambda: f.exec('E', 'test', '-f', RUN + '/unsolicited-sent', check=False).returncode == 0)
            time.sleep(.5)
            f.check('selected exit cannot initiate unsolicited UDP', f.read('C', 'udp.json')['received'] == 0)
            # Raw physical routing through E must not be treated as Mesh authorization.
            f.exec('C', 'ip', 'route', 'add', '172.29.233.21/32', 'via', '172.29.231.12')
            f.check('E is not a non-Mesh open router', f.http('C', '172.29.233.21')['exit_code'] != 0)
            before = f.read('S', 'udp.json')['received']
            f.exec('C', 'python3', '-c', "import socket; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.sendto(b'not-mesh',('172.29.233.21',9001))")
            time.sleep(.5)
            f.check('non-Mesh packet never reaches external server', f.read('S', 'udp.json')['received'] == before)
    elif case == 'unauthorized':
        f.setup(authorized=False)
        f.check('source without internet grant rejected', f.http('C', '172.29.233.20')['exit_code'] != 0)
    elif case == 'internet-only':
        f.setup(default=True, internet_only=True)
        response = f.http('C', '172.29.233.20')
        f.check('internet-only grant cannot NAT into exit LAN', response['exit_code'] != 0, response=response)
        route = json.loads(f.exec('C', 'ip', '-j', 'route', 'get', '172.29.233.20').stdout)
        f.check('private destination is captured but authorization-denied at E', route[0]['dev'] == 'mwexit0', route=route)
        f.check('E can reach its LAN independently', f.http('E', '172.29.233.20')['exit_code'] == 0)
    elif case == 'default':
        f.setup(default=True)
        f.succeeds('172.29.233.21', '172.29.233.10')
        route = json.loads(f.exec('C', 'ip', '-j', 'route', 'get', '172.29.231.12').stdout)
        f.check('underlay remains pinned outside TUN', route[0]['dev'] == 'eth0', route=route)
        f.check('default exit denies implicit local LAN access', f.http('C', '172.29.231.20')['exit_code'] != 0)
        ipv6 = f.exec('C', 'ip', '-6', 'route', 'get', '2001:db8::1', check=False)
        rules = json.loads(f.exec('C', 'ip', '-j', '-6', 'rule').stdout)
        routes = json.loads(f.exec('C', 'ip', '-j', '-6', 'route', 'show', 'table', '12570').stdout)
        f.check('IPv6 default is fail closed', ipv6.returncode != 0 and any(r.get('priority') == 12570 for r in rules)
                and any(r.get('type') == 'unreachable' and r.get('dst') == 'default' for r in routes),
                stderr=ipv6.stderr, rules=rules, routes=routes)
    elif case in ['withdraw', 'lease', 'kill-client', 'kill-exit']:
        f.setup(default=True, lease=14 if case == 'lease' else 180)
        f.succeeds('172.29.233.20', '172.29.233.10')
        name = f.flow()
        entries = f.exec('E', 'conntrack', '-L', '--mark', '0x4d570001', check=False)
        f.check('live stream has marked kernel conntrack state', 'ESTABLISHED' in entries.stdout, entries=entries.stdout)
        if case == 'withdraw':
            f.touch('E', 'withdraw')
            wait('exit withdrawn', lambda: f.exec('E', 'test', '-f', RUN + '/stopped', check=False).returncode == 0)
        elif case == 'lease':
            wait('lease watchdog (no HTTP refresh)', lambda: f.exec('E', 'test', '-f', RUN + '/stopped', check=False).returncode == 0)
        else:
            f.kill('C' if case == 'kill-client' else 'E')
        f.flow_stopped(name)
        f.check('new external stream fails after interruption', f.http('C', '172.29.233.20')['exit_code'] != 0)
        if case in ['withdraw', 'lease']:
            entries = f.exec('E', 'conntrack', '-L', '--mark', '0x4d570001', check=False)
            f.check('only owned marked conntrack state cleared', not entries.stdout.strip(), entries=entries.stdout)
        if case == 'kill-client':
            route = f.exec('C', 'ip', '-j', 'route', 'get', '172.29.231.20', check=False)
            f.check('SIGKILL retains client sink rather than local fallback', route.returncode != 0, stderr=route.stderr)
    elif case == 'direct-exclude':
        f.setup(default=True, direct_count=1000)
        f.exec('L', 'ip', 'address', 'add', '10.128.0.1/32', 'dev', 'eth0')
        f.exec('C', 'ip', 'route', 'add', '10.128.0.1/32', 'dev', 'eth0')
        local = f.http('C', '10.128.0.1')
        f.check('direct exclusion under default exit is local', local['exit_code'] == 0 and json.loads(local['stdout'])['source'] == '172.29.231.10', response=local)
        f.succeeds('172.29.233.20', '172.29.233.10')
        routes = json.loads(f.exec('C', 'ip', '-j', '-4', 'route', 'show', 'table', '12570').stdout)
        excluded = [r for r in routes if r.get('type') == 'throw' and r.get('dst', '').startswith('10.128.')]
        f.check('1000 exclusions installed in owned table', len(excluded) == 1000, install=f.read('C', 'install.json'), count=len(excluded))
        # Correctness across the full set; no address may resolve to the TUN.
        sample = f.exec('C', 'python3', '-c', "import ipaddress,json,subprocess; failures=[]; base=int(ipaddress.ip_address('10.128.0.0'));\nfor n in range(1000):\n p=subprocess.run(['ip','-j','route','get',str(ipaddress.ip_address(base+n))],capture_output=True,text=True);\n if p.returncode==0 and any(r.get('dev')=='mwexit0' for r in json.loads(p.stdout)): failures.append(n)\nprint(json.dumps({'captured':failures}))", timeout=120)
        f.check('all 1000 direct prefixes avoid TUN', not json.loads(sample.stdout)['captured'])
        previous = f.read('C', 'ready')['generation']
        f.touch('C', 'replace-request')
        wait('replacement generation active', lambda: f.read('C', 'ready')['generation'] > previous)
        routes = json.loads(f.exec('C', 'ip', '-j', '-4', 'route', 'show', 'table', '12572').stdout)
        f.check('1000-prefix fenced atomic generation replacement',
                len([r for r in routes if r.get('type') == 'throw' and r.get('dst', '').startswith('10.129.')]) == 1000,
                replacement=f.read('C', 'replace.json'))
        f.exec('L', 'ip', 'address', 'add', '10.129.0.1/32', 'dev', 'eth0')
        f.exec('C', 'ip', 'route', 'add', '10.129.0.1/32', 'dev', 'eth0')
        f.check('replacement direct prefix is local', f.http('C', '10.129.0.1')['exit_code'] == 0)
        f.check('retired direct prefix now follows default exit',
                json.loads(f.exec('C', 'ip', '-j', 'route', 'get', '10.128.0.1').stdout)[0]['dev'] == 'mwexit0')
        f.succeeds('172.29.233.20', '172.29.233.10')
        f.touch('C', 'restore-local')
        wait('explicit capture cancellation', lambda: f.exec('C', 'test', '-f', RUN + '/stopped', check=False).returncode == 0)
        rules = json.loads(f.exec('C', 'ip', '-j', '-4', 'rule').stdout)
        f.check('1000-prefix explicit withdrawal removes owned capture', not any(r.get('priority') in [12560, 12568, 12570] for r in rules), removal=f.read('C', 'remove.json'))
    elif case in ['dual', 'locked']:
        f.setup(mode='dual', locked=case == 'locked')
        e, other = 'e', 'f'
        f.succeeds('172.29.233.21', '172.29.233.10')
        f.selection(f'{99:064x}', accept=False)
        if case == 'locked':
            f.selection(other, accept=False)
            f.selection('mesh', accept=False)
            f.succeeds('172.29.233.21', '172.29.233.10')
        else:
            before = f.read('S', 'udp.json')['received']
            f.touch('C', 'inject-wrong-exit')
            wait('authenticated public-route bypass attempt', lambda: f.exec('C', 'test', '-f', RUN + '/wrong-exit-sent', check=False).returncode == 0)
            time.sleep(.5)
            f.check('alternate allowed exit cannot bypass public destination path', f.read('S', 'udp.json')['received'] == before)
            name = f.flow(target='172.29.233.21')
            f.selection(other)
            f.flow_stopped(name)
            f.succeeds('172.29.233.21', '172.29.233.11')
            f.check('profile switch replaces specific direct table too', f.http('C', '172.29.231.20')['exit_code'] == 0)
            f.succeeds('172.29.233.20', '172.29.233.10')
            f.selection('mesh')
            f.check('cancel default restores local route', f.http('C', '172.29.231.20')['exit_code'] == 0)
            f.succeeds('172.29.233.20', '172.29.233.10')
            f.selection(e)
            f.succeeds('172.29.233.21', '172.29.233.10')
            f.check('retired profile direct exclusion is removed', f.http('C', '172.29.231.20')['exit_code'] != 0)
            f.selection('auto')
            previous = f.read('C', 'ready')['generation']
            f.exec('C', 'python3', '-c', 'from pathlib import Path; import sys; Path(sys.argv[1]).write_text("JP")', RUN + '/auto-resolution')
            wait('controller fixture location update', lambda: f.read('C', 'ready')['generation'] > previous)
            state = f.read('C', 'ready')
            f.check('auto uses controller-resolved location profile', state['selected_profile'] == 'auto' and state['effective_profile'] == 'f', observation=state)
            f.succeeds('172.29.233.21', '172.29.233.11')
            previous = state['generation']
            f.exec('C', 'python3', '-c', 'from pathlib import Path; import sys; Path(sys.argv[1]).write_text("unknown")', RUN + '/auto-resolution')
            wait('unknown location fallback', lambda: f.read('C', 'ready')['generation'] > previous)
            f.check('unknown auto location uses configured fallback', f.read('C', 'ready')['effective_profile'] == 'mesh', observation=f.read('C', 'ready'))
            f.check('auto fallback restores local route', f.http('C', '172.29.231.20')['exit_code'] == 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--cases', nargs='+', choices=['direct', 'relay', 'unauthorized', 'internet-only', 'default', 'withdraw', 'lease', 'kill-client', 'kill-exit', 'dual', 'locked', 'direct-exclude'],
                        default=['direct', 'relay', 'unauthorized', 'internet-only', 'default', 'withdraw', 'lease', 'kill-client', 'kill-exit', 'dual', 'locked', 'direct-exclude'])
    args = parser.parse_args()
    os.umask(0o077)
    args.artifacts.mkdir(parents=True, exist_ok=True)
    output = args.artifacts / 'result.json'
    if output.exists():
        raise ValueError('result.json already exists')
    result = {'run_id': 'meshway-exit-' + uuid.uuid4().hex[:12], 'status': 'running', 'cleanup_errors': [], 'cases': [],
              'started_at': datetime.now(timezone.utc).isoformat(), 'source_revision': command('git', 'rev-parse', 'HEAD').stdout.strip(),
              'source_dirty': bool(command('git', 'status', '--porcelain').stdout.strip()),
              'scope': 'Ignored fixture injection into production internal ExitPlan, shared Runtime/E2E/packet_io and Linux TUN/NAT; no controller enrollment.'}
    active = None
    def save():
        temporary = output.with_suffix('.tmp')
        temporary.write_text(json.dumps(result, indent=2))
        temporary.replace(output)
    def interrupted(*_):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    save()
    try:
        info = json.loads(docker('info', '--format', '{{json .}}').stdout)
        if info['OSType'] != 'linux':
            raise RuntimeError('Linux Docker required')
        result['docker'] = {key: info.get(key) for key in ['ServerVersion', 'Architecture', 'KernelVersion', 'NCPU']}
        result['image'] = json.loads(docker('image', 'inspect', args.image).stdout)[0]['Id']
        for case in args.cases:
            active = Fixture(args, result, case)
            exercise(active, case)
            result['cleanup_errors'].extend(active.cleanup())
            active = None
            save()
            if result['cleanup_errors']:
                raise RuntimeError('Owned cleanup failed')
        result['status'] = 'ok'
    except KeyboardInterrupt:
        result['status'] = 'interrupted'
    except Exception as error:
        result.update(status='failed', error=str(error))
    finally:
        if active:
            # Collect public diagnostics only, never private fixture JSON.
            result['failure_diagnostics'] = {}
            for node in active.nodes:
                result['failure_diagnostics'][node] = {kind: active.exec(node, *cmd, check=False).stdout for kind, cmd in {
                    'routes': ['ip', '-j', '-4', 'route', 'show', 'table', 'all'],
                    'rules': ['ip', '-j', '-4', 'rule'], 'nft': ['nft', 'list', 'ruleset'], 'log': ['cat', RUN + '/agent.log']}.items()}
            result['cleanup_errors'].extend(active.cleanup())
        result['finished_at'] = datetime.now(timezone.utc).isoformat()
        if result['cleanup_errors']:
            result['status'] = 'failed'
        save()
    return 0 if result['status'] == 'ok' else 1


if __name__ == '__main__':
    # A wrapper/launcher descriptor is reused, never recursively acquired.
    with performance_lock(inherited_env='MESHWAY_EXIT_PERF_LOCK_FD') as descriptor:
        LOCK_DESCRIPTOR = descriptor
        sys.exit(main())
