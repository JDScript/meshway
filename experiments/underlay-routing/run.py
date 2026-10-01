#!/usr/bin/env python3
"""Host orchestration; all Docker calls require the inherited shared lock."""
import argparse
from datetime import datetime, timezone
import json
import hashlib
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import uuid

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'transport'))
from performance_lock import LOCK_FD_ENV, locked_launch, validate_descriptor

TMP = Path('/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode')
LABEL = 'meshway.underlay.owner'


def docker(*args, check=True, timeout=60):
    descriptor = int(os.environ[LOCK_FD_ENV])
    validate_descriptor(descriptor)
    p = subprocess.run(['docker', *args], capture_output=True, text=True,
                       timeout=timeout, pass_fds=(descriptor,))
    if check and p.returncode:
        raise RuntimeError(f'docker {args[:3]}: {p.stderr[-2000:]}')
    return p


def owns(inspect, run_id, network=False):
    if not isinstance(inspect, list) or len(inspect) != 1 or not isinstance(inspect[0], dict):
        return False
    config = inspect[0] if network else inspect[0].get('Config')
    if not isinstance(config, dict):
        return False
    labels = config.get('Labels')
    return isinstance(labels, dict) and labels.get(LABEL) == run_id


def run(args):
    run_id = 'mw-underlay-' + uuid.uuid4().hex[:12]
    directory = args.artifacts.resolve() / run_id
    if not directory.is_relative_to(TMP) or directory == TMP:
        raise ValueError('artifacts must be under approved private TMP')
    directory.mkdir(mode=0o700, parents=True)
    result = {'run_id': run_id, 'started_at': datetime.now(timezone.utc).isoformat(),
              'status': 'failed', 'checks': [], 'cleanup_errors': [], 'image': args.image}
    containers, networks = [], []

    def terminate(signum, _frame):
        raise RuntimeError(f'terminated by signal {signum}')

    previous = {sig: signal.signal(sig, terminate) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        if not args.skip_build:
            docker('build', '-t', args.image, str(HERE), timeout=600)
        result['image_id'] = json.loads(docker('image', 'inspect', args.image).stdout)[0]['Id']
        networks.append(run_id)
        docker('network', 'create', '--internal', '--ipv6', '--subnet', '172.30.247.0/24',
               '--subnet', 'fd74:6d77:247::/64', '--label', LABEL + '=' + run_id, run_id)
        for node, ip4, ip6 in (('server', '172.30.247.20', 'fd74:6d77:247::20'),
                                ('client', '172.30.247.10', 'fd74:6d77:247::10')):
            name = run_id + '-' + node
            containers.append(name)
            docker('run', '-d', '--name', name, '--label', LABEL + '=' + run_id,
                   '--network', run_id, '--ip', ip4, '--ip6', ip6,
                   '--cap-drop', 'ALL', '--cap-add', 'NET_ADMIN', '--cap-add', 'SETUID', '--cap-add', 'SETGID',
                   '--device', '/dev/net/tun', '--security-opt', 'no-new-privileges', '--read-only',
                   '--tmpfs', '/run:rw,nosuid,nodev,size=16m', '--pids-limit', '64', '--memory', '128m',
                   '--sysctl', 'net.ipv6.conf.all.disable_ipv6=0',
                   '--entrypoint', 'python3', args.image, '-c', 'import signal; signal.pause()')
        docker('exec', '-d', containers[0], 'python3', '/lab/fixture.py', 'server')
        actual = json.loads(docker('exec', containers[1], 'python3', '-c',
                            'import hashlib,json,pathlib; print(json.dumps({n:hashlib.sha256('
                            'pathlib.Path("/lab",n).read_bytes()).hexdigest() '
                            'for n in ("helper.py","fixture.py")}))').stdout)
        expected = {n: hashlib.sha256((HERE / n).read_bytes()).hexdigest() for n in actual}
        result['fixture_source_sha256'] = actual
        if actual != expected:
            raise RuntimeError('stale image fixture sources; rebuild without --skip-build')
        result['kernel'] = docker('exec', containers[1], 'uname', '-srvm').stdout.strip()
        # Wait for actual marked HTTP response (no capture yet), not a fixed sleep.
        ready = False
        for _ in range(40):
            p = docker('exec', containers[1], 'python3', '-c',
                       'import socket; s=socket.create_connection(("172.30.247.20",41861),.2); '
                       's.sendall(b"GET / HTTP/1.1\\r\\n\\r\\n"); print(s.recv(4096))', check=False)
            if p.returncode == 0:
                ready = True
                break
            time.sleep(.1)
        if not ready:
            raise RuntimeError('fixture server did not become ready')
        p = docker('exec', containers[1], 'python3', '/lab/fixture.py', 'client', check=False, timeout=120)
        (directory / 'client.stdout').write_text(p.stdout)
        (directory / 'client.stderr').write_text(p.stderr)
        parsed = json.loads(p.stdout)
        result.update(parsed)
        if p.returncode != 0:
            result['status'] = 'failed'
        logs = docker('exec', containers[0], 'ip', '-j', 'address', 'show').stdout
        result['server_addresses'] = json.loads(logs)
        observer = docker('exec', containers[0], 'cat', '/run/observer.jsonl').stdout
        (directory / 'observer.jsonl').write_text(observer)
        result['observer_events'] = [json.loads(line) for line in observer.splitlines()]
        if result['status'] == 'ok':
            sent = result['pre_retirement_send']
            witnessed = [event for event in result['observer_events']
                         if event.get('transaction') == sent['transaction']
                         and event['recvfrom'][:2] == sent['local'][:2]]
            result['checks'].append({'name': 'already-sent STUN survives local retirement fence at observer',
                                     'status': 'ok' if witnessed else 'failed', 'observer_events': witnessed})
            if not witnessed:
                result['status'] = 'failed'
    except BaseException as error:
        result['status'], result['error'] = 'failed', str(error)
    finally:
        # Forwarded termination should not interrupt precise child-owned cleanup.
        for sig in previous:
            signal.signal(sig, signal.SIG_IGN)
        for name in reversed(containers):
            try:
                p = docker('inspect', name, check=False)
                if p.returncode:
                    if 'No such' not in p.stderr:
                        result['cleanup_errors'].append(p.stderr)
                    continue
                if not owns(json.loads(p.stdout), run_id):
                    result['cleanup_errors'].append('refused foreign container ' + name)
                    continue
                p = docker('rm', '-f', name, check=False)
                if p.returncode:
                    result['cleanup_errors'].append(p.stderr)
            except Exception as error:
                result['cleanup_errors'].append(f'container {name}: {error}')
        for name in reversed(networks):
            try:
                p = docker('network', 'inspect', name, check=False)
                if p.returncode:
                    if 'not found' not in p.stderr and 'No such' not in p.stderr:
                        result['cleanup_errors'].append(p.stderr)
                    continue
                if not owns(json.loads(p.stdout), run_id, network=True):
                    result['cleanup_errors'].append('refused foreign network ' + name)
                    continue
                p = docker('network', 'rm', name, check=False)
                if p.returncode:
                    result['cleanup_errors'].append(p.stderr)
            except Exception as error:
                result['cleanup_errors'].append(f'network {name}: {error}')
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        if result['cleanup_errors']:
            result['status'] = 'failed'
        result['finished_at'] = datetime.now(timezone.utc).isoformat()
        (directory / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({'status': result['status'], 'result': str(directory / 'result.json'),
                          'checks': len(result['checks']), 'cleanup_errors': result['cleanup_errors']}), flush=True)
    return 0 if result['status'] == 'ok' else 1


if __name__ == '__main__':
    # Wrapper retains exactly one shared lock through build, run, and cleanup.
    if '--child' not in sys.argv:
        sys.exit(locked_launch([sys.executable, str(HERE / 'run.py'), '--child', *sys.argv[1:]]))
    parser = argparse.ArgumentParser()
    parser.add_argument('--child', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--image', default='meshway-underlay-fixture:local')
    parser.add_argument('--artifacts', type=Path, default=TMP / 'meshway-underlay-artifacts')
    sys.exit(run(parser.parse_args()))
