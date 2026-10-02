#!/usr/bin/env python3
"""F1/F2 and persisted new-scope regressions in owned, isolated Docker netns."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
import signal
from pathlib import Path
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


def run(args):
    owner = 'mw-underlay-review-fix-' + uuid.uuid4().hex[:12]
    directory = mechanism.TMP / 'meshway-underlay-artifacts' / owner
    directory.mkdir(parents=True, mode=0o700)
    result = {'status': 'failed', 'owner': owner, 'started_at': datetime.now(timezone.utc).isoformat(),
              'cleanup_errors': [], 'cases': [], 'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'source_sha256': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                  for p in sorted((ROOT / 'apps/agent/src/underlay').glob('*.rs'))}}
    result['source_sha256']['apps/agent/src/managed_dataplane/tun.rs'] = hashlib.sha256((ROOT / 'apps/agent/src/managed_dataplane/tun.rs').read_bytes()).hexdigest()
    result['dependency_sha256'] = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in (
        'Cargo.toml', 'Cargo.lock', 'crates/vendor/README.md', 'crates/vendor/socket2-upstream-sha256.json', 'crates/vendor/socket2/src/sys/unix.rs')}
    result['fixture_sha256'] = {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest() for name in ('review.py', 'Dockerfile')}
    containers, networks, volumes = [], [], []
    docker = mechanism.docker
    previous = {}
    def stop(signum, frame):
        raise RuntimeError(f'terminated: {signum}')
    for sig in (signal.SIGINT, signal.SIGTERM):
        previous[sig] = signal.signal(sig, stop)

    def case(container, name, extra=(), module='review_tests', ignored=True):
        output = docker('exec', '-e', 'MESHWAY_UNDERLAY_FIXTURE=1', *extra, container, '/underlay-tests', '--exact',
                       'underlay::owner::' + module + '::' + name, *(['--ignored'] if ignored else []), '--nocapture', check=False, timeout=90)
        row = {'name': name, 'exit': output.returncode, 'stdout': output.stdout, 'stderr': output.stderr}
        result['cases'].append(row)
        (directory / f'{len(result["cases"]):02}-{name}.json').write_text(json.dumps(row, indent=2))
        if output.returncode:
            raise RuntimeError(f'{name} failed: {output.stderr}')
        assert 'running 1 test' in output.stdout, f'{name} did not execute an actual test'

    def create(name, network, volume=None, tun=False, server=False):
        containers.append(name)
        flags = ['--network', network] if network.startswith('container:') else [
            '--network', network, '--ip', '172.30.247.20' if server else '172.30.247.10', '--ip6', 'fd74:6d77:247::20' if server else 'fd74:6d77:247::10',
            '--sysctl', 'net.ipv6.conf.default.disable_ipv6=1' if tun else 'net.ipv6.conf.all.disable_ipv6=0', '--sysctl', 'net.ipv4.ip_forward=1', '--sysctl', 'net.ipv6.conf.all.forwarding=1']
        if volume:
            flags += ['--mount', f'type=volume,source={volume},target=/state']
        docker('run', '-d', '--name', name, '--label', mechanism.LABEL + '=' + owner,
               *flags, '--cap-drop', 'ALL', '--cap-add', 'NET_ADMIN', '--device', '/dev/net/tun',
               '--security-opt', 'no-new-privileges', '--read-only', '--tmpfs', '/run:rw,nosuid,nodev,size=64m',
               '--tmpfs', '/etc/iproute2:rw,nosuid,nodev,size=1m', '--pids-limit', '128', '--memory', '256m',
               '--entrypoint', 'python3', args.image, '-c', 'import signal; signal.pause()')

    try:
        if not args.skip_build:
            builder_name = docker('context', 'show').stdout.strip()
            builder = docker('buildx', 'inspect', builder_name).stdout
            result['builder'] = builder
            assert any(line.split() == ['Driver:', 'docker'] for line in builder.splitlines()), 'default builder must be ordinary docker driver'
            docker('build', '--builder', builder_name, '-f', str(HERE / 'Dockerfile'), '-t', args.image, str(ROOT), timeout=1200)
        result['image_id'] = json.loads(docker('image', 'inspect', args.image).stdout)[0]['Id']
        source_name = owner + '-sources'
        containers.append(source_name)
        actual = json.loads(docker('run', '--rm', '--name', source_name, '--label', mechanism.LABEL + '=' + owner,
            '--network', 'none', '--cap-drop', 'ALL', '--read-only', '--entrypoint', 'python3', args.image, '-c',
            'import pathlib,hashlib,json; s={"apps/agent/src/underlay/"+p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in pathlib.Path("/sources/underlay").glob("*.rs")}; s["apps/agent/src/managed_dataplane/tun.rs"]=hashlib.sha256(pathlib.Path("/sources/tun.rs").read_bytes()).hexdigest(); print(json.dumps(s))').stdout)
        assert actual == result['source_sha256'], 'image source bytes do not match checkout'
        network = owner + '-network'
        networks.append(network)
        docker('network', 'create', '--internal', '--ipv6', '--subnet', '172.30.247.0/24', '--subnet', 'fd74:6d77:247::/64',
               '--label', mechanism.LABEL + '=' + owner, network)
        client = owner + '-client'
        create(client, network)
        case(client, 'cookie_identity_has_no_inode_or_socket_fallback', module='cookie_tests', ignored=False)
        result['cookie_observations'] = []
        for _ in range(2):
            case(client, 'cookie_observe', module='cookie_tests')
            line = next(line for line in result['cases'][-1]['stdout'].splitlines() if line.startswith('NETNS_COOKIE_OBSERVATION '))
            result['cookie_observations'].append(json.loads(line.split(' ', 1)[1]))
        assert result['cookie_observations'][0] == result['cookie_observations'][1], 'same namespace cookie changed across processes'
        case(client, 'legacy_and_damage_refused', module='cookie_tests')
        unsupported = docker('exec', '-e', 'MESHWAY_UNDERLAY_FIXTURE=1', client, 'strace',
            '-f', '-e', 'trace=getsockopt', '-e', 'inject=getsockopt:error=ENOPROTOOPT:when=1', '-o', '/run/cookie-unsupported.log',
            '/underlay-tests', '--exact', 'underlay::owner::cookie_tests::cookie_unsupported', '--ignored', '--nocapture', check=False, timeout=30)
        result['cookie_unsupported'] = {'exit': unsupported.returncode, 'stdout': unsupported.stdout, 'stderr': unsupported.stderr}
        unsupported_trace = docker('exec', client, 'cat', '/run/cookie-unsupported.log').stdout
        (directory / 'cookie-unsupported.log').write_text(unsupported_trace)
        assert unsupported.returncode == 0 and 'SO_NETNS_COOKIE' in unsupported_trace and 'ENOPROTOOPT' in unsupported_trace, result['cookie_unsupported']
        for boundary in ['archived', 'fresh-journal']:
            case(client, 'scope_crash_stage', ['-e', 'MESHWAY_UNDERLAY_SCOPE_BOUNDARY=' + boundary,
                '-e', 'MESHWAY_UNDERLAY_TEST_SCOPE_EXIT=' + boundary], module='cookie_tests')
            case(client, 'scope_crash_recover', ['-e', 'MESHWAY_UNDERLAY_SCOPE_BOUNDARY=' + boundary], module='cookie_tests')
        case(client, 'rollover_guard_refused', module='cookie_tests')
        case(client, 'legacy_incomplete_topology_refused')
        case(client, 'cancellation_boundaries')
        for kind, count in [('reservation', 4), ('mesh', 2), ('capture', 4)]:
            for boundary in range(1, count + 1):
                case(client, 'operation_crash_stage', ['-e', 'MESHWAY_UNDERLAY_TEST_OPERATION=' + kind,
                      '-e', 'MESHWAY_UNDERLAY_TEST_COMMIT=' + str(boundary)])
                case(client, 'operation_crash_recover', ['-e', 'MESHWAY_UNDERLAY_TEST_RECOVER_KIND=' + kind])
        docker('exec', '-d', '-e', 'MESHWAY_UNDERLAY_FIXTURE=1', client, 'sh', '-c',
               '/underlay-tests --exact underlay::owner::review_tests::singleton_hold --ignored --nocapture > /run/singleton-holder.log 2>&1; echo $? > /run/singleton-holder.exit')
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if docker('exec', client, 'test', '-f', '/run/underlay-singleton-ready', check=False).returncode == 0:
                break
            time.sleep(.05)
        else:
            raise RuntimeError('singleton holder did not become ready')
        other = owner + '-private-mount'
        create(other, 'container:' + client)
        result['singleton_views'] = {name: docker('exec', name, 'python3', '-c',
            'import os,json; print(json.dumps({"netns":os.stat("/proc/self/ns/net").st_ino,"run":os.stat("/run").st_ino,"mount":os.stat("/proc/self/ns/mnt").st_ino}))').stdout.strip()
            for name in (client, other)}
        case(other, 'singleton_contender')
        docker('exec', client, 'touch', '/run/underlay-singleton-release')
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            exit_code = docker('exec', client, 'cat', '/run/singleton-holder.exit', check=False)
            if exit_code.returncode == 0:
                assert exit_code.stdout.strip() == '0', exit_code.stdout
                break
            time.sleep(.05)
        else:
            raise RuntimeError('singleton holder did not release/restore ownership')
        result['singleton_holder'] = docker('exec', client, 'cat', '/run/singleton-holder.log').stdout
        # Remove only our owned containers before recreating the namespace.
        for name in [other, client]:
            assert mechanism.owns(json.loads(docker('inspect', name).stdout), owner)
            docker('rm', '-f', name)
        volume = owner + '-state'
        volumes.append(volume)
        docker('volume', 'create', '--label', mechanism.LABEL + '=' + owner, volume)
        first = owner + '-scope-one'
        create(first, network, volume, tun=True)
        case(first, 'scope_create')
        assert mechanism.owns(json.loads(docker('inspect', first).stdout), owner)
        docker('rm', '-f', first)
        second = owner + '-scope-two'
        create(second, network, volume, tun=True)
        case(second, 'cookie_observe', module='cookie_tests')
        second_cookie = json.loads(next(line.split(' ', 1)[1] for line in result['cases'][-1]['stdout'].splitlines() if line.startswith('NETNS_COOKIE_OBSERVATION ')))
        assert second_cookie['cookie'] != result['cookie_observations'][0]['cookie'], 'different namespace lifetime reused cookie'
        result['second_namespace_cookie'] = second_cookie
        server = owner + '-tun-server'
        create(server, network, server=True)
        docker('exec', '-d', server, 'python3', '/lab/production.py', 'server')
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if docker('exec', server, 'test', '-f', '/run/ready', check=False).returncode == 0:
                break
            time.sleep(.05)
        else:
            raise RuntimeError('controller fixture server did not start')
        case(second, 'scope_fresh')
        assert mechanism.owns(json.loads(docker('inspect', second).stdout), owner)
        docker('rm', '-f', second)
        tun_client = owner + '-tun-client'
        create(tun_client, network, tun=True)
        case(tun_client, 'normal_open_cancel', module='tun_recovery_tests')
        for kind, boundaries in [('open', range(1, 5)), ('activate', [0])]:
            for boundary in boundaries:
                case(tun_client, 'normal_open_crash', ['-e', 'MESHWAY_UNDERLAY_TEST_OPERATION=' + kind,
                    '-e', 'MESHWAY_UNDERLAY_TEST_COMMIT=' + str(boundary)], module='tun_recovery_tests')
                case(tun_client, 'normal_open_recover', module='tun_recovery_tests')
        result['status'] = 'ok'
    except BaseException as error:
        result['error'] = str(error)
    finally:
        for sig in previous:
            signal.signal(sig, signal.SIG_IGN)
        for name in reversed(containers):
            try:
                found = docker('inspect', name, check=False)
                if found.returncode == 0:
                    assert mechanism.owns(json.loads(found.stdout), owner)
                    docker('rm', '-f', name)
            except Exception as error:
                result['cleanup_errors'].append(str(error))
        for name in reversed(volumes):
            try:
                found = json.loads(docker('volume', 'inspect', name).stdout)[0]
                assert found.get('Labels', {}).get(mechanism.LABEL) == owner
                docker('volume', 'rm', name)
            except Exception as error:
                result['cleanup_errors'].append(str(error))
        for name in reversed(networks):
            try:
                found = json.loads(docker('network', 'inspect', name).stdout)
                assert mechanism.owns(found, owner, network=True)
                docker('network', 'rm', name)
            except Exception as error:
                result['cleanup_errors'].append(str(error))
        if result['cleanup_errors']:
            result['status'] = 'failed'
        result['finished_at'] = datetime.now(timezone.utc).isoformat()
        destination = directory / 'result.json'
        destination.write_text(json.dumps(result, indent=2) + '\n')
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        print(json.dumps({'status': result['status'], 'result': str(destination), 'error': result.get('error'), 'cleanup_errors': result['cleanup_errors']}), flush=True)
    return 0 if result['status'] == 'ok' else 1


if __name__ == '__main__':
    if '--child' not in sys.argv:
        signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError('bounded review fixture timeout')))
        signal.alarm(2400)
        sys.exit(locked_launch([sys.executable, str(HERE / 'review.py'), '--child', *sys.argv[1:]]))
    parser = argparse.ArgumentParser()
    parser.add_argument('--child', action='store_true')
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--image', default='meshway-underlay-review-fix:local')
    sys.exit(run(parser.parse_args()))
