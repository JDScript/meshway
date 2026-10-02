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
sys.path.insert(0, str(ROOT / 'experiments/transport'))
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
    result['fixture_sha256'] = {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest() for name in ('review.py', 'Dockerfile')}
    containers, networks, volumes = [], [], []
    docker = mechanism.docker
    previous = {}
    def stop(signum, frame):
        raise RuntimeError(f'terminated: {signum}')
    for sig in (signal.SIGINT, signal.SIGTERM):
        previous[sig] = signal.signal(sig, stop)

    def case(container, name, extra=()):
        output = docker('exec', '-e', 'MESHWAY_UNDERLAY_FIXTURE=1', *extra, container, '/underlay-tests', '--exact',
                       'underlay::owner::review_tests::' + name, '--ignored', '--nocapture', check=False, timeout=90)
        row = {'name': name, 'exit': output.returncode, 'stdout': output.stdout, 'stderr': output.stderr}
        result['cases'].append(row)
        (directory / f'{len(result["cases"]):02}-{name}.json').write_text(json.dumps(row, indent=2))
        if output.returncode:
            raise RuntimeError(f'{name} failed: {output.stderr}')
        assert 'running 1 test' in output.stdout, f'{name} did not execute an actual test'

    def create(name, network, volume=None):
        containers.append(name)
        flags = ['--network', network] if network.startswith('container:') else [
            '--network', network, '--ip', '172.30.247.10', '--ip6', 'fd74:6d77:247::10',
            '--sysctl', 'net.ipv6.conf.all.disable_ipv6=0', '--sysctl', 'net.ipv4.ip_forward=1', '--sysctl', 'net.ipv6.conf.all.forwarding=1']
        if volume:
            flags += ['--mount', f'type=volume,source={volume},target=/state']
        docker('run', '-d', '--name', name, '--label', mechanism.LABEL + '=' + owner,
               *flags, '--cap-drop', 'ALL', '--cap-add', 'NET_ADMIN', '--device', '/dev/net/tun',
               '--security-opt', 'no-new-privileges', '--read-only', '--tmpfs', '/run:rw,nosuid,nodev,size=64m',
               '--tmpfs', '/etc/iproute2:rw,nosuid,nodev,size=1m', '--pids-limit', '128', '--memory', '256m',
               '--entrypoint', 'python3', args.image, '-c', 'import signal; signal.pause()')

    try:
        if not args.skip_build:
            docker('build', '-f', str(HERE / 'Dockerfile'), '-t', args.image, str(ROOT), timeout=1200)
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
        create(first, network, volume)
        case(first, 'scope_create')
        assert mechanism.owns(json.loads(docker('inspect', first).stdout), owner)
        docker('rm', '-f', first)
        second = owner + '-scope-two'
        create(second, network, volume)
        case(second, 'scope_fresh')
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
