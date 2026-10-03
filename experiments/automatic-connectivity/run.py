#!/usr/bin/env python3
"""Isolated normal Controller/Agent automatic-adjacency acceptance (not NAT proof)."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
TMP = Path('/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode')
sys.path.insert(0, str(ROOT / 'scripts'))
sys.path.insert(0, str(ROOT / 'apps/agent/tests/underlay_fixture'))
from performance_lock import performance_lock, validate_descriptor, LOCK_FD_ENV
from source_manifest import inventory
from topology import accept
from build import build


def run(args):
    name = 'mw-auto-' + uuid.uuid4().hex[:12]
    logs = TMP / 'meshway-automatic-connectivity-runs' / name
    logs.mkdir(parents=True, mode=0o700)
    result = {'owner': name, 'status': 'failed', 'cleanup_errors': [], 'commands': []}
    source, context = (logs / child for child in ('source', 'image'))
    work = Path(args.build_work).resolve() if args.build_work else logs / 'work'
    for directory in (source, work, context, work / 'cargo', work / 'home', work / 'target'):
        directory.mkdir(mode=0o700, exist_ok=True)
    manifest = inventory(ROOT)
    for relative in manifest['files']:
        target = source / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, target)
    assert inventory(source) == manifest
    result['source_head'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    result['source_manifest'] = manifest
    result['fixture_sha256'] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                                for p in sorted(Path(__file__).parent.glob('*.py'))}
    if args.cargo_cache and not args.image and not (work / 'cargo/registry').exists():
        try:
            shutil.copytree(Path(args.cargo_cache).resolve() / 'registry', work / 'cargo/registry')
        except OSError as error:
            result['error'] = str(error)
            (logs / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
            print(json.dumps({'status': 'failed', 'result': str(logs / 'result.json'), 'error': str(error)}))
            return 1
    owned, networks = [], []
    with performance_lock() as descriptor:
        def docker(*command, check=True, timeout=120, input=None):
            validate_descriptor(descriptor)
            safe = list(map(str, command))
            for i, value in enumerate(safe[:-1]):
                if value == '--token':
                    safe[i + 1] = '<private>'
            result['commands'].append(safe)
            completed = subprocess.run(['docker', *map(str, command)], cwd=ROOT,
                env=dict(os.environ, DOCKER_CONTEXT='default', BUILDX_BUILDER='default',
                         **{LOCK_FD_ENV: str(descriptor)}), pass_fds=(descriptor,),
                text=True, capture_output=True, timeout=timeout, input=input)
            if check and completed.returncode:
                raise RuntimeError(completed.stderr[-4000:] + completed.stdout[-4000:])
            return completed
        try:
            image = args.image or build(docker, args, name, logs, source, work, context, manifest, result, owned)
            result['image'] = json.loads(docker('image', 'inspect', image).stdout)[0]['Id']

            def container(role, network, agent=True):
                cid = docker('create', '--name', name + '-' + role, '--label', 'meshway.automatic.acceptance=' + name,
                    '--network', network, '--network-alias', role + '.mesh.test', '--cap-drop', 'ALL',
                    *(['--cap-add', 'NET_ADMIN', '--device', '/dev/net/tun',
                       '--sysctl', 'net.ipv6.conf.default.disable_ipv6=1'] if agent else []),
                    '--security-opt', 'no-new-privileges', '--read-only', '--cpus', '1', '--memory', '384m',
                    '--pids-limit', '128', '--tmpfs', '/run:rw,nosuid,nodev,mode=0700,size=128m',
                    '--tmpfs', '/tmp:rw,nosuid,nodev,size=64m', '--env', 'TMPDIR=/run', image).stdout.strip()
                owned.append(cid)
                inspected = json.loads(docker('inspect', cid).stdout)[0]
                assert not inspected['HostConfig']['Privileged'] and inspected['HostConfig']['NetworkMode'] != 'host'
                assert not inspected['HostConfig']['PortBindings'], 'endpoints may not publish ports'
                (logs / (role + '-inspect.json')).write_text(json.dumps(inspected, indent=2))
                docker('start', cid)
                actual = json.loads(docker('exec', cid, 'cat', '/source-manifest.json').stdout)
                assert actual == manifest
                hashes = docker('exec', cid, 'sha256sum', '/usr/local/bin/meshway-agent', '/usr/local/bin/meshway-controller').stdout
                binaries = {line.split()[1]: line.split()[0] for line in hashes.splitlines()}
                assert result.setdefault('binary_sha256', binaries) == binaries
                return cid

            accept(docker, container, name, logs, result, networks)
            result['status'] = 'passed'
        except BaseException as error:
            result['error'] = str(error)
        finally:
            handlers = {sig: signal.signal(sig, signal.SIG_IGN) for sig in (signal.SIGINT, signal.SIGTERM)}
            for cid in reversed(owned):
                try:
                    inspected = json.loads(docker('inspect', cid).stdout)[0]
                    assert inspected['Config']['Labels'].get('meshway.automatic.acceptance') == name
                    for label in ('agent', 'controller'):
                        completed = docker('exec', cid, 'cat', '/run/' + label + '.log', check=False)
                        if completed.returncode == 0:
                            (logs / (inspected['Name'].strip('/') + '-' + label + '.log')).write_text(completed.stdout)
                    docker('rm', '-f', cid)
                except Exception as error:
                    result['cleanup_errors'].append(str(error))
            for network in reversed(networks):
                try:
                    inspected = json.loads(docker('network', 'inspect', network).stdout)[0]
                    assert inspected['Labels'].get('meshway.automatic.acceptance') == name
                    docker('network', 'rm', network)
                except Exception as error:
                    result['cleanup_errors'].append(str(error))
            (logs / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
            for sig, handler in handlers.items():
                signal.signal(sig, handler)
    print(json.dumps({'status': result['status'], 'result': str(logs / 'result.json'), 'error': result.get('error')}))
    return int(result['status'] != 'passed' or bool(result['cleanup_errors']))


if __name__ == '__main__':
    def terminate(_signal, _frame):
        raise KeyboardInterrupt('termination requested; cleaning owned resources')
    signal.signal(signal.SIGTERM, terminate)
    parser = argparse.ArgumentParser()
    parser.add_argument('--compiler', default='meshway-underlay-compiler:mw-underlay-431-60aab4705c43')
    parser.add_argument('--runtime', help='coordinator-prepared runtime image for a new build')
    parser.add_argument('--image', help='reuse only an image whose complete Rust source manifest matches this checkout')
    parser.add_argument('--cargo-cache', help='read-only source of registry cache; copied into isolated build work')
    parser.add_argument('--build-work', help='private compiler cache reused serially under the shared Docker lock')
    args = parser.parse_args()
    if not args.image and not args.runtime:
        parser.error('one of --runtime or --image is required')
    sys.exit(run(args))
