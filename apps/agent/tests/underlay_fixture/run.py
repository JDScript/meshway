#!/usr/bin/env python3
"""Build and exercise actual Rust code under the shared, bounded Docker lock."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import uuid

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / 'experiments/underlay-routing'))
import run as mechanism
sys.path.insert(0, str(ROOT / 'experiments/transport'))
from performance_lock import locked_launch
sys.path.insert(0, str(HERE))
from trace import audit


def run(args):
    owner = 'mw-rust-underlay-' + uuid.uuid4().hex[:12]
    directory = mechanism.TMP / 'meshway-underlay-artifacts' / owner
    directory.mkdir(mode=0o700, parents=True)
    result = {'owner': owner, 'status': 'failed', 'cleanup_errors': [], 'started_at': datetime.now(timezone.utc).isoformat(),
              'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'source_diff_sha256': hashlib.sha256(subprocess.check_output(['git', 'diff', 'HEAD'], cwd=ROOT)).hexdigest(),
              'fixture_source_sha256': {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest()
                                        for name in ['namespace.py', 'run.py', 'Dockerfile', 'trace.py']}}
    containers, networks, transients = [], [], []
    docker = mechanism.docker
    previous = {}
    def stop(signum, frame):
        raise RuntimeError(f'terminated: {signum}')
    for sig in (signal.SIGTERM, signal.SIGINT):
        previous[sig] = signal.signal(sig, stop)
    try:
        if not args.skip_build:
            docker('build', '-f', str(HERE / 'Dockerfile'), '-t', args.image, str(ROOT), timeout=1200)
        result['image_id'] = json.loads(docker('image', 'inspect', args.image).stdout)[0]['Id']
        result['source_sha256'] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                                  for p in sorted((ROOT / 'apps/agent/src/underlay').glob('*.rs'))}
        source_check = owner + '-sources'
        transients.append(source_check)
        actual_sources = json.loads(docker('run', '--rm', '--name', source_check, '--label', mechanism.LABEL + '=' + owner, '--network', 'none', '--cap-drop', 'ALL', '--read-only', '--entrypoint', 'python3', args.image, '-c',
            'import hashlib,json,pathlib; print(json.dumps({"apps/agent/src/underlay/"+p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in pathlib.Path("/sources/underlay").glob("*.rs")}))').stdout)
        if actual_sources != result['source_sha256']:
            raise RuntimeError('image Rust sources do not match checkout; rebuild')
        networks.append(owner)
        docker('network', 'create', '--internal', '--ipv6', '--subnet', '172.30.247.0/24', '--subnet', 'fd74:6d77:247::/64', '--label', mechanism.LABEL + '=' + owner, owner)
        for role, v4, v6 in [('server', '.20', '::20'), ('client', '.10', '::10')]:
            name = owner + '-' + role
            containers.append(name)
            docker('run', '-d', '--name', name, '--label', mechanism.LABEL + '=' + owner,
                   '--network', owner, '--ip', '172.30.247' + v4, '--ip6', 'fd74:6d77:247' + v6,
                   '--cap-drop', 'ALL', '--cap-add', 'NET_ADMIN', '--cap-add', 'SETUID', '--cap-add', 'SETGID',
                   '--device', '/dev/net/tun', '--security-opt', 'no-new-privileges', '--read-only',
                   '--tmpfs', '/run:rw,nosuid,nodev,size=64m', '--pids-limit', '128', '--memory', '256m',
                   '--tmpfs', '/etc/iproute2:rw,nosuid,nodev,size=1m',
                   '--sysctl', 'net.ipv6.conf.all.disable_ipv6=0', '--entrypoint', 'python3', args.image,
                   '-c', 'import signal; signal.pause()')
        docker('exec', '-d', containers[0], 'python3', '/lab/production.py', 'server')
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            ready = docker('exec', containers[0], 'test', '-f', '/run/ready', check=False)
            dad = docker('exec', containers[1], 'ip', '-6', '-j', 'address', 'show').stdout
            if ready.returncode == 0 and 'tentative' not in dad:
                break
            time.sleep(.1)
        else:
            raise RuntimeError('observer or IPv6 DAD not ready')
        child = docker('exec', containers[1], 'python3', '/lab/production.py', 'client', check=False, timeout=120)
        (directory / 'client.stdout').write_text(child.stdout)
        (directory / 'client.stderr').write_text(child.stderr)
        result.update(json.loads(child.stdout))
        if child.returncode:
            result['status'] = 'failed'
        syscall = docker('exec', containers[1], 'cat', '/run/syscalls.log').stdout
        (directory / 'syscalls.log').write_text(syscall)
        observer = docker('exec', containers[0], 'cat', '/run/observer.jsonl').stdout
        (directory / 'observer.jsonl').write_text(observer)
        events = [json.loads(line) for line in observer.splitlines()]
        result['observer_events'] = events
        if result['status'] == 'ok':
            for version in (4, 6):
                for port in (41861, 41862, 41864):
                    assert any(e.get('service') == 'accept' and e['version'] == version and e['port'] == port for e in events), (version, port)
                assert any(e.get('service') == 'listener-synack' and e['version'] == version for e in events)
            assert all(e.get('payload') != b'must-not-send'.hex() for e in events)
            assert sum(e.get('service') == 'dns' and e.get('hostname') == 'tcp.underlay.fixture' and e.get('transport') == 'tcp' for e in events) == 8
            for path in ('/slow', '/headers'):
                assert any(e.get('service') == 'retirement-eof' and e.get('path') == path and e.get('eof') for e in events), path
            assert 'SO_MARK' in syscall and 'setsockopt' in syscall and 'getsockopt' in syscall
            result['syscall_audit'] = audit(syscall)
            assert result['syscall_audit']['premarked_listeners'] == 4
    except BaseException as error:
        result['status'], result['error'] = 'failed', str(error)
    finally:
        for sig in previous:
            signal.signal(sig, signal.SIG_IGN)
        for name in reversed(containers + transients):
            try:
                found = docker('inspect', name, check=False)
                if found.returncode == 0:
                    if not mechanism.owns(json.loads(found.stdout), owner):
                        raise RuntimeError('refused foreign container ' + name)
                    docker('rm', '-f', name)
            except Exception as error:
                result['cleanup_errors'].append(str(error))
        for name in reversed(networks):
            try:
                found = docker('network', 'inspect', name, check=False)
                if found.returncode == 0:
                    if not mechanism.owns(json.loads(found.stdout), owner, network=True):
                        raise RuntimeError('refused foreign network ' + name)
                    docker('network', 'rm', name)
            except Exception as error:
                result['cleanup_errors'].append(str(error))
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        if result['cleanup_errors']:
            result['status'] = 'failed'
        result['finished_at'] = datetime.now(timezone.utc).isoformat()
        (directory / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({'status': result['status'], 'result': str(directory / 'result.json'), 'error': result.get('error'), 'cleanup_errors': result['cleanup_errors']}), flush=True)
    return 0 if result['status'] == 'ok' else 1


if __name__ == '__main__':
    if '--child' not in sys.argv:
        # Bounded wait/build/run. launch forwards termination and awaits precise
        # child cleanup; the one inherited descriptor covers all Docker calls.
        signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError('bounded underlay fixture timeout')))
        signal.alarm(2400)
        sys.exit(locked_launch([sys.executable, str(HERE / 'run.py'), '--child', *sys.argv[1:]]))
    parser = argparse.ArgumentParser()
    parser.add_argument('--child', action='store_true')
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--image', default='meshway-rust-underlay-fixture:local')
    sys.exit(run(parser.parse_args()))
