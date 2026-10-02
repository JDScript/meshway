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
sys.path.insert(0, str(ROOT / 'scripts'))
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
        result['source_sha256']['apps/agent/src/managed_dataplane/tun.rs'] = hashlib.sha256((ROOT / 'apps/agent/src/managed_dataplane/tun.rs').read_bytes()).hexdigest()
        source_check = owner + '-sources'
        transients.append(source_check)
        actual_sources = json.loads(docker('run', '--rm', '--name', source_check, '--label', mechanism.LABEL + '=' + owner, '--network', 'none', '--cap-drop', 'ALL', '--read-only', '--entrypoint', 'python3', args.image, '-c',
            'import hashlib,json,pathlib; s={"apps/agent/src/underlay/"+p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in pathlib.Path("/sources/underlay").glob("*.rs")}; s["apps/agent/src/managed_dataplane/tun.rs"]=hashlib.sha256(pathlib.Path("/sources/tun.rs").read_bytes()).hexdigest(); print(json.dumps(s))').stdout)
        if actual_sources != result['source_sha256']:
            raise RuntimeError('image Rust sources do not match checkout; rebuild')
        networks.append(owner)
        docker('network', 'create', '--internal', '--ipv6', '--subnet', '172.30.247.0/24', '--subnet', 'fd74:6d77:247::/64', '--label', mechanism.LABEL + '=' + owner, owner)
        for role, v4, v6 in [('server', '.20', '::20'), ('client', '.10', '::10')]:
            name = owner + '-' + role
            containers.append(name)
            docker('run', '-d', '--name', name, '--label', mechanism.LABEL + '=' + owner,
                   '--network', owner, '--ip', '172.30.247' + v4, '--ip6', 'fd74:6d77:247' + v6,
                   '--network-alias', 'underlay-owner-' + role, '--add-host', 'owner-host:172.30.247.20',
                   '--cap-drop', 'ALL', '--cap-add', 'NET_ADMIN', '--cap-add', 'SETUID', '--cap-add', 'SETGID',
                   '--device', '/dev/net/tun', '--security-opt', 'no-new-privileges', '--read-only',
                   '--tmpfs', '/run:rw,nosuid,nodev,size=64m', '--pids-limit', '128', '--memory', '256m',
                   '--tmpfs', '/etc/iproute2:rw,nosuid,nodev,size=1m',
                   '--sysctl', 'net.ipv6.conf.all.disable_ipv6=0', '--entrypoint', 'python3', args.image,
                   '-c', 'import signal; signal.pause()')
        # Normal IPv4 TUN inherits disabled IPv6, while the real physical underlay
        # stays dual-stack. This is the existing Linux TUN safety prerequisite,
        # configured only in this disposable namespace (never a host sysctl).
        containers.append(owner + '-tun')
        docker('run', '-d', '--name', containers[2], '--label', mechanism.LABEL + '=' + owner,
               '--network', owner, '--ip', '172.30.247.12', '--ip6', 'fd74:6d77:247::12',
               '--cap-drop', 'ALL', '--cap-add', 'NET_ADMIN', '--device', '/dev/net/tun',
               '--security-opt', 'no-new-privileges', '--read-only', '--tmpfs', '/run:rw,nosuid,nodev,size=64m',
               '--tmpfs', '/etc/iproute2:rw,nosuid,nodev,size=1m', '--pids-limit', '128', '--memory', '256m',
               '--sysctl', 'net.ipv6.conf.default.disable_ipv6=1',
               '--entrypoint', 'python3', args.image,
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
        docker('exec', '-d', containers[0], 'sh', '-c', 'MESHWAY_UNDERLAY_FIXTURE=1 /underlay-tests --exact underlay::fixture::native_peer --ignored --nocapture > /run/native-peer.log 2>&1; echo $? > /run/native-peer.exit')
        child = docker('exec', containers[1], 'python3', '/lab/production.py', 'client', check=False, timeout=220)
        (directory / 'client.stdout').write_text(child.stdout)
        (directory / 'client.stderr').write_text(child.stderr)
        result.update(json.loads(child.stdout))
        if child.returncode:
            result['status'] = 'failed'
        result['normal_tun_ipv6_environment'] = json.loads(docker('exec', containers[2], 'python3', '-c',
            'import json,pathlib; print(json.dumps({name:pathlib.Path("/proc/sys/net/ipv6/conf/"+name+"/disable_ipv6").read_text().strip() for name in ("all","default","eth0")}))').stdout)
        normal_tun = docker('exec', '-e', 'MESHWAY_UNDERLAY_FIXTURE=1', containers[2], '/underlay-tests', '--exact',
            'underlay::owner::tests::namespace_normal_tun_preparation', '--ignored', '--nocapture', check=False, timeout=60)
        result['normal_tun_preparation'] = {'exit': normal_tun.returncode, 'stdout': normal_tun.stdout, 'stderr': normal_tun.stderr}
        (directory / 'normal-tun.stdout').write_text(normal_tun.stdout)
        (directory / 'normal-tun.stderr').write_text(normal_tun.stderr)
        if normal_tun.returncode:
            result['status'] = 'failed'
            result.setdefault('error', 'normal TUN ownership preparation failed')
        syscall = docker('exec', containers[1], 'cat', '/run/syscalls.log', check=False).stdout
        (directory / 'syscalls.log').write_text(syscall)
        activation_trace = docker('exec', containers[1], 'cat', '/run/syscalls-activation.log', check=False).stdout
        (directory / 'syscalls-activation.log').write_text(activation_trace)
        if result['status'] == 'ok':
            result['activation_syscall_audit'] = audit(activation_trace, ordinary_connects=2, ordinary_binds=0, mark=0x80000000, mask=0x80000000)
        ur2_syscall = docker('exec', containers[1], 'cat', '/run/syscalls-ur2.log', check=False).stdout
        (directory / 'syscalls-ur2.log').write_text(ur2_syscall)
        for name in ('native', 'managed'):
            trace = docker('exec', containers[1], 'cat', f'/run/syscalls-{name}.log', check=False).stdout
            (directory / f'syscalls-{name}.log').write_text(trace)
            if result['status'] == 'ok':
                result[f'{name}_syscall_audit'] = audit(trace, ordinary_connects=0, ordinary_binds=0)
        peer = docker('exec', containers[0], 'cat', '/run/native-peer.log', check=False).stdout
        (directory / 'native-peer.log').write_text(peer)
        peer_exit = docker('exec', containers[0], 'cat', '/run/native-peer.exit', check=False).stdout.strip()
        result['native_peer_exit'] = peer_exit
        if result['status'] == 'ok' and peer_exit != '0':
            raise RuntimeError('native peer did not finish successfully')
        observer = docker('exec', containers[0], 'cat', '/run/observer.jsonl', check=False).stdout
        (directory / 'observer.jsonl').write_text(observer)
        events = [json.loads(line) for line in observer.splitlines()]
        result['observer_events'] = events
        if result['status'] == 'ok':
            for version in (4, 6):
                for port in (41861, 41862, 41864):
                    assert any(e.get('service') == 'accept' and e['version'] == version and e['port'] == port for e in events), (version, port)
                assert any(e.get('service') == 'listener-synack' and e['version'] == version for e in events)
            assert all(e.get('payload') != b'must-not-send'.hex() for e in events)
            assert sum(e.get('service') == 'native-stun' for e in events) == 4
            assert sum(e.get('service') == 'dns' and e.get('hostname') == 'tcp.underlay.fixture' and e.get('transport') == 'tcp' for e in events) == 8
            for path in ('/slow', '/headers'):
                assert any(e.get('service') == 'retirement-eof' and e.get('path') == path and e.get('eof') for e in events), path
            assert 'SO_MARK' in syscall and 'setsockopt' in syscall and 'getsockopt' in syscall
            result['syscall_audit'] = audit(syscall)
            assert result['syscall_audit']['premarked_listeners'] == 4
            result['ur2_syscall_audit'] = audit(ur2_syscall, ordinary_connects=0, ordinary_binds=0)
            result['ur2_observer_checks'] = []
            for version in (4, 6):
                for hostname, primary, backup, tcp, failing in [
                    ('backup.underlay.fixture', 2, 2, 0, 0),
                    ('tcpbackup.underlay.fixture', 2, 2, 2, 0),
                    ('allfail.underlay.fixture', 2, 0, 0, 2),
                    ('nxdomainbackup.underlay.fixture', 2, 0, 0, 0),
                    ('refused.underlay.fixture', 2, 0, 0, 0),
                    ('cancelbackup.underlay.fixture', 2, 0, 0, 0),
                ]:
                    actual = [e for e in events if e.get('service') == 'dns' and e.get('hostname') == hostname and e.get('version') == version]
                    counts = (sum(e['port'] == 41866 for e in actual), sum(e['port'] == 41864 and e['transport'] == 'udp' for e in actual),
                              sum(e['port'] == 41864 and e['transport'] == 'tcp' for e in actual), sum(e['port'] == 41867 for e in actual))
                    assert counts == (primary, backup, tcp, failing), (version, hostname, counts)
                    result['ur2_observer_checks'].append({'version': version, 'hostname': hostname, 'counts': counts, 'status': 'ok'})
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
