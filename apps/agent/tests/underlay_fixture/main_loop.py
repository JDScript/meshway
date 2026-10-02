#!/usr/bin/env python3
"""Normal Agent underlay acceptance, with ordinary bounded Linux containers."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[4]
TMP = Path('/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode')
sys.path.insert(0, str(ROOT / 'scripts'))
from performance_lock import performance_lock, validate_descriptor, LOCK_FD_ENV
from source_manifest import inventory


def run(args):
    name = 'mw-underlay-main-loop-' + uuid.uuid4().hex[:12]
    logs = TMP / 'meshway-underlay-main-loop-runs' / name
    logs.mkdir(parents=True, mode=0o700)
    work = TMP / 'meshway-underlay-main-loop-linux'
    work.mkdir(mode=0o700, exist_ok=True)
    for child in ('cargo', 'home', 'target'):
        (work / child).mkdir(exist_ok=True)
    if not (work / 'cargo/registry').exists():
        cache = TMP / 'meshway-underlay-integration-4312037-logs/linux-work/cargo/registry'
        if cache.exists():
            shutil.copytree(cache, work / 'cargo/registry')
    result = {'owner': name, 'status': 'failed', 'cleanup_errors': [], 'commands': [], 'inspect': []}
    source = logs / 'source'
    source.mkdir()
    for relative in ('Cargo.toml', 'Cargo.lock', '.dockerignore', 'crates', 'apps/controller', 'apps/agent', 'apps/apple/core',
                     'experiments/underlay-routing/helper.py', 'experiments/underlay-routing/fixture.py'):
        origin, destination = ROOT / relative, source / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        if origin.is_dir():
            shutil.copytree(origin, destination, ignore=shutil.ignore_patterns('target', 'node_modules', '__pycache__'))
        else:
            shutil.copy2(origin, destination)
    result['source_head'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    result['source_sha256'] = {str(p.relative_to(source)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(source.rglob('*')) if p.is_file()}
    with performance_lock() as descriptor:
        def docker(*command, check=True, timeout=120, input=None):
            validate_descriptor(descriptor)
            safe = list(command)
            for i, value in enumerate(safe[:-1]):
                if value == '--token': safe[i + 1] = '<private>'
            result['commands'].append(safe)
            completed = subprocess.run(['docker', *map(str, command)], cwd=ROOT,
                env=dict(os.environ, DOCKER_CONTEXT='default', BUILDX_BUILDER='default',
                    **{LOCK_FD_ENV: str(descriptor)}), pass_fds=(descriptor,),
                text=True, capture_output=True, timeout=timeout, input=input)
            if check and completed.returncode:
                raise RuntimeError(completed.stderr[-4000:] + completed.stdout[-4000:])
            return completed
        owned, networks = [], []
        try:
            builder = docker('buildx', 'inspect', 'default').stdout
            assert 'Driver:        docker' in builder or any(line.split() == ['Driver:', 'docker'] for line in builder.splitlines())
            (logs / 'builder.txt').write_text(builder)
            command = 'cargo check --locked -p meshway-agent --all-features --tests'
            if not args.check_only:
                command = ('cargo clippy --locked -p meshway-agent --all-features --all-targets -- -D warnings && '
                    'cargo test --locked -p meshway-agent --all-features --lib --no-run --message-format=json > /work/tests.json && '
                    'cargo build --locked -p meshway-agent -p meshway-controller --all-features')
            created = docker('create', '--name', name + '-compile', '--label', 'meshway.underlay.acceptance=' + name,
                '--user', f'{os.getuid()}:{os.getgid()}', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                '--read-only', '--cpus', '2', '--memory', '4g', '--pids-limit', '512', '--network', 'bridge',
                '--tmpfs', '/tmp:rw,nosuid,nodev,size=256m',
                '--mount', f'type=bind,source={source},target=/source,readonly',
                '--mount', f'type=bind,source={work},target=/work', '--workdir', '/source',
                '--env', 'HOME=/work/home', '--env', 'CARGO_HOME=/work/cargo', '--env', 'CARGO_TARGET_DIR=/work/target',
                '--env', 'CARGO_BUILD_JOBS=2', '--env', 'RUSTUP_TOOLCHAIN=1.98.0',
                '--entrypoint', 'sh', args.compiler, '-c', command)
            cid = created.stdout.strip()
            owned.append(cid)
            inspection = json.loads(docker('inspect', cid).stdout)[0]
            result['inspect'].append(inspection)
            host = inspection['HostConfig']
            assert not host['Privileged'] and host['NetworkMode'] != 'host' and 'ALL' in host['CapDrop'] and not host.get('CapAdd')
            assert inspection['Config']['User'] == f'{os.getuid()}:{os.getgid()}'
            compiled = docker('start', '-a', cid, check=False, timeout=1800)
            (logs / 'compile.log').write_text(compiled.stdout + compiled.stderr)
            result['compile_exit'] = json.loads(docker('inspect', cid).stdout)[0]['State']['ExitCode']
            assert result['compile_exit'] == 0, 'Linux compile failed; see compile.log'
            result['status'] = 'compiled'
            if not args.check_only and not args.compile_only:
                context = logs / 'runtime'
                context.mkdir()
                for binary in ('meshway-agent', 'meshway-controller'):
                    shutil.copy2(work / 'target/debug' / binary, context / binary)
                artifacts = [json.loads(line) for line in (work / 'tests.json').read_text().splitlines() if line.startswith('{')]
                tests = [item['executable'] for item in artifacts if item.get('reason') == 'compiler-artifact'
                    and item.get('executable') and item['target']['name'] == 'meshway_agent']
                assert len(tests) == 1
                shutil.copy2(work / Path(tests[0]).relative_to('/work'), context / 'underlay-tests')
                (context / 'source-manifest.json').write_text(json.dumps(inventory(source), sort_keys=True))
                for relative, output in (
                    ('apps/agent/tests/underlay_fixture/namespace.py', 'production.py'),
                    ('experiments/underlay-routing/helper.py', 'helper.py'),
                    ('experiments/underlay-routing/fixture.py', 'fixture.py'),
                ):
                    shutil.copy2(source / relative, context / output)
                (context / 'Dockerfile').write_text(f'FROM {args.runtime}\n'
                    'COPY --chmod=755 meshway-agent meshway-controller /usr/local/bin/\n'
                    'COPY --chmod=755 underlay-tests /underlay-tests\n'
                    'COPY source-manifest.json /source-manifest.json\n'
                    'COPY production.py helper.py fixture.py /lab/\n'
                    'ENTRYPOINT ["python3", "-c", "import signal; signal.pause()"]\n')
                image = 'meshway-underlay-main-loop:' + name
                docker('build', '--builder', 'default', '-t', image, str(context), timeout=600)
                result['image'] = json.loads(docker('image', 'inspect', image).stdout)[0]['Id']

                def container(role, network, alias, extra=(), agent=True):
                    cid = docker('create', '--name', name + '-' + role, '--label', 'meshway.underlay.acceptance=' + name,
                        '--network', network, '--network-alias', alias, '--cap-drop', 'ALL',
                        *(['--cap-add', 'NET_ADMIN', '--device', '/dev/net/tun',
                            '--sysctl', 'net.ipv6.conf.default.disable_ipv6=1'] if agent else []),
                        '--security-opt', 'no-new-privileges', '--read-only', '--cpus', '1', '--memory', '384m',
                        '--pids-limit', '128', '--tmpfs', '/run:rw,nosuid,nodev,mode=0700,size=128m',
                        '--tmpfs', '/tmp:rw,nosuid,nodev,size=64m', '--env', 'TMPDIR=/run',
                        *extra, image).stdout.strip()
                    owned.append(cid)
                    inspection = json.loads(docker('inspect', cid).stdout)[0]
                    result['inspect'].append(inspection)
                    assert not inspection['HostConfig']['Privileged'] and inspection['HostConfig']['NetworkMode'] != 'host'
                    docker('start', cid)
                    return cid

                fixture_net = name + '-fixtures'
                docker('network', 'create', '--internal', '--label', 'meshway.underlay.acceptance=' + name, fixture_net)
                networks.append(fixture_net)
                result['behavior_tests'] = []
                for case in ('pending', 'lost', 'old', 'binding-failure'):
                    cid = container('test-' + case, fixture_net, 'test-' + case)
                    completed = docker('exec', '-e', 'MESHWAY_UNDERLAY_FIXTURE=1', '-e', 'MESHWAY_UNDERLAY_CASE=' + case,
                        cid, '/underlay-tests', '--exact', 'control::protection::tests::controller_bootstrap_recovers_identity_and_disabled_scope',
                        '--ignored', '--nocapture', check=False, timeout=90)
                    (logs / ('startup-' + case + '.log')).write_text(completed.stdout + completed.stderr)
                    result['behavior_tests'].append({'case': case, 'exit': completed.returncode})
                    assert completed.returncode == 0, 'protected startup case failed: ' + case
                    assert '1 passed' in completed.stdout, 'startup test was not collected'
                for case in ('capability', 'foreign', 'orphan-capture', 'foreign-tun', 'bound-journal', 'lifetime', 'damage', 'cleanup'):
                    cid = container('test-' + case, fixture_net, 'test-' + case)
                    completed = docker('exec', '-e', 'MESHWAY_UNDERLAY_FIXTURE=1', '-e', 'MESHWAY_UNDERLAY_CASE=' + case,
                        cid, '/underlay-tests', '--exact', 'underlay::owner::bootstrap::tests::guard_only_capability_requires_retirement_and_preserves_foreign_resources',
                        '--ignored', '--nocapture', check=False, timeout=90)
                    (logs / ('bootstrap-' + case + '.log')).write_text(completed.stdout + completed.stderr)
                    result['behavior_tests'].append({'case': case, 'exit': completed.returncode})
                    assert completed.returncode == 0, 'bootstrap boundary case failed: ' + case
                    assert '1 passed' in completed.stdout, 'bootstrap test was not collected'
                normal_acceptance(docker, container, name, logs, result, networks)
                result['status'] = 'passed'
        except BaseException as error:
            result['status'] = 'failed'
            result['error'] = str(error)
        finally:
            # A second termination request must not interrupt resource cleanup
            # or release the lock while its Docker children are still running.
            handlers = {sig: signal.signal(sig, signal.SIG_IGN)
                for sig in (signal.SIGINT, signal.SIGTERM)}
            for cid in reversed(owned):
                try:
                    inspection = json.loads(docker('inspect', cid).stdout)[0]
                    assert inspection['Config']['Labels'].get('meshway.underlay.acceptance') == name
                    if not args.check_only and not args.compile_only and 'compile' not in inspection['Name']:
                        for label in ('agent', 'controller'):
                            observed = docker('exec', cid, 'cat', '/run/' + label + '.log', check=False)
                            if observed.returncode == 0:
                                (logs / (inspection['Name'].strip('/') + '-' + label + '.log')).write_text(observed.stdout)
                        observed = docker('exec', cid, 'ip', '-N', '-j', '-4', 'route', 'show', 'table', 'all', check=False)
                        if observed.returncode == 0:
                            (logs / (inspection['Name'].strip('/') + '-routes.json')).write_text(observed.stdout)
                    docker('rm', '-f', cid)
                except Exception as error:
                    result['cleanup_errors'].append(str(error))
            for network in reversed(networks):
                try:
                    inspection = json.loads(docker('network', 'inspect', network).stdout)[0]
                    assert inspection['Labels'].get('meshway.underlay.acceptance') == name
                    docker('network', 'rm', network)
                except Exception as error:
                    result['cleanup_errors'].append(str(error))
            (logs / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
            for sig, handler in handlers.items():
                signal.signal(sig, handler)
    print(json.dumps({'status': result['status'], 'result': str(logs / 'result.json'), 'error': result.get('error')}))
    expected = 'compiled' if args.check_only or args.compile_only else 'passed'
    return int(result['status'] != expected or bool(result['cleanup_errors']))


def normal_acceptance(docker, container, name, logs, result, networks):
    """Normal binaries, controller-issued policy, and ordinary HTTP through TUN."""
    for suffix, subnet in (('control', '100.96.1.0/24'), ('carrier', None)):
        network = name + '-' + suffix
        docker('network', 'create', '--internal', '--label', 'meshway.underlay.acceptance=' + name,
            *(['--subnet', subnet] if subnet else []), network)
        networks.append(network)
    control_net, carrier_net = networks[-2:]
    controller = container('controller', control_net, 'controller.mesh.test', ('--ip', '100.96.1.10'), agent=False)
    agents = {role: container(role, carrier_net, role + '.mesh.test') for role in ('agent-a', 'agent-b')}
    for role, cid in agents.items():
        docker('network', 'connect', '--alias', role + '-control.mesh.test', control_net, cid)

    def spawn(cid, label, argv):
        code = ('import json,os,pathlib,subprocess,sys; os.umask(0o077); a=json.load(sys.stdin); '
            'p=subprocess.Popen(a,stdin=subprocess.DEVNULL,stdout=open("/run/' + label + '.log","ab"),stderr=subprocess.STDOUT); '
            'pathlib.Path("/run/' + label + '.pid").write_text(str(p.pid))')
        docker('exec', '-i', cid, 'python3', '-c', code, input=json.dumps(argv))

    def stop(cid, label, signal=2):
        docker('exec', cid, 'python3', '-c',
            f'import os,pathlib; os.kill(int(pathlib.Path("/run/{label}.pid").read_text()),{signal})')

    def until(description, function, seconds=75):
        deadline, last = time.monotonic() + seconds, None
        while time.monotonic() < deadline:
            try:
                value = function()
                if value: return value
            except Exception as error:
                last = str(error)
            time.sleep(.5)
        raise RuntimeError(description + ': ' + str(last))

    spawn(controller, 'controller', ['/usr/local/bin/meshway-controller', 'serve', '--bind', '0.0.0.0:8420', '--data-dir', '/run/controller'])
    admin = until('controller initialization', lambda: docker('exec', controller, '/usr/local/bin/meshway-controller',
        'admin-token', '--data-dir', '/run/controller').stdout.strip(), 20)
    rpc_code = ('import http.client,json,sys; d=json.load(sys.stdin); c=http.client.HTTPConnection("127.0.0.1",8420,timeout=5); '
        'b=None if d["body"] is None else json.dumps(d["body"]); '
        'c.request("GET" if b is None else "POST",d["path"],b,{"Authorization":"Bearer "+d["token"],"Content-Type":"application/json"}); '
        'r=c.getresponse(); s=r.read(); assert r.status==200,(r.status,s.decode()); print(s.decode())')
    def rpc(path, body=None):
        return json.loads(docker('exec', '-i', controller, 'python3', '-c', rpc_code,
            input=json.dumps({'token': admin, 'path': path, 'body': body})).stdout)
    network = until('controller API readiness', lambda: rpc('/api/v1/networks'), 20)['networks'][0]['network_id']
    prefix = '/api/v1/networks/' + network
    mesh_prefix = rpc(prefix + '/policy')['network']['ipv4_prefix']
    nodes = {}
    for role, cid in agents.items():
        token = rpc(prefix + '/enrollment-tokens', {'expires_in_seconds': 300})['token']
        enrolled = docker('exec', cid, '/usr/local/bin/meshway-agent', 'enroll', '--controller', 'http://controller.mesh.test:8420',
            '--name', role, '--token', token, '--state', '/run/agent.json')
        (logs / (role + '-enroll.log')).write_text(enrolled.stdout + enrolled.stderr)
        status = json.loads(docker('exec', cid, '/usr/local/bin/meshway-agent', 'status', '--state', '/run/agent.json').stdout)
        nodes[role] = status['node_id']
        spawn(cid, 'agent', ['/usr/local/bin/meshway-agent', 'run', '--state', '/run/agent.json', '--apply-dataplane',
            '--interval-seconds', '2', '--connection-lease-seconds', '20', '--dns-listen', '127.0.0.1:5353'])
    inventory = until('normal key publication', lambda: (lambda s: s if all(any(n['node']['id'] == node and n['public_key']
        for n in s['nodes']) for node in nodes.values()) else False)(rpc(prefix + '/connections')))
    a, b = nodes['agent-a'], nodes['agent-b']
    addresses = {entry['node']['id']: entry['node']['mesh_ipv4'] for entry in inventory['nodes']}
    rpc(prefix + '/connections', {'expected_revision': inventory['revision'], 'configuration': {
        'listeners': [{'id': 'normal-udp', 'node_id': b, 'transport': 'native_udp', 'bind': '0.0.0.0:43101',
            'advertised_endpoints': [{'host': 'agent-b.mesh.test', 'port': 43101}], 'credential_ref': None}],
        'candidates': [{'id': 'normal-dial', 'dialer_node_id': a, 'listener_id': 'normal-udp',
            'endpoint': {'host': 'agent-b.mesh.test', 'port': 43101}, 'priority': 0, 'credential_ref': None}]}})
    rules = [{'id': 'normal-' + str(i), 'source': {'node_ids': [source], 'countries': []}, 'destination_scope': 'mesh',
        'destination_any': [{'kind': 'node', 'value': destination}], 'action': {'kind': 'route', 'via': [], 'exit_node_id': None}}
        for i, (source, destination) in enumerate(((a, b), (b, a)))]
    rpc(prefix + '/policy/save', {'expected_revision': rpc(prefix + '/policy')['revision'],
        'policy': {'default_action': 'reject', 'rules': rules}, 'source_jsonc': None})
    rpc(prefix + '/access', {'expected_revision': rpc(prefix + '/access')['revision'], 'policy': {
        'default_action': 'reject', 'rules': [{'id': 'normal-grant', 'source': ['node:' + a], 'destination': ['node:' + b],
            'action': 'allow', 'bidirectional': True}]}, 'source_jsonc': None})

    for role, cid in agents.items():
        ip = addresses[nodes[role]]
        until(role + ' TUN assignment', lambda cid=cid, ip=ip: ip in docker('exec', cid, 'ip', '-j', '-4', 'address', 'show', 'dev', 'meshway0').stdout)
        spawn(cid, 'business', ['python3', '-m', 'http.server', '8088', '--bind', ip, '--directory', '/run'])
    probe_code = ('import http.client,sys; c=http.client.HTTPConnection(sys.argv[1],8088,timeout=2); '
        'c.request("GET","/"); r=c.getresponse(); assert r.status==200; r.read(); print("real-tun-http")')
    def traffic():
        for source, destination in ((a, b), (b, a)):
            cid = agents['agent-a' if source == a else 'agent-b']
            completed = docker('exec', cid, 'python3', '-c', probe_code, addresses[destination], check=False, timeout=6)
            if completed.returncode: return False
        return True
    until('bidirectional real TUN HTTP', traffic)
    result['normal_tun'] = {'nodes': nodes, 'addresses': addresses, 'bidirectional_http': True}
    # Controller address deliberately lies inside the Mesh prefix. Only its
    # internal marked sockets may reach main; ordinary business must enter TUN.
    ordinary = docker('exec', agents['agent-a'], 'python3', '-c',
        'import socket; s=socket.socket(); assert s.getsockopt(socket.SOL_SOCKET,36)==0; s.settimeout(2); s.connect(("100.96.1.10",8420))',
        check=False, timeout=5)
    assert ordinary.returncode != 0, 'ordinary controller-address socket bypassed Mesh capture'
    result['normal_tun']['same_controller_ordinary_denied'] = True
    stop(controller, 'controller', 9)
    time.sleep(26)
    for role, cid in agents.items():
        destination = addresses[b if nodes[role] == a else a]
        lookup = docker('exec', cid, 'ip', '-4', 'route', 'get', destination, check=False)
        assert lookup.returncode != 0, 'lease expiry leaked Mesh to underlay'
        rows = json.loads(docker('exec', cid, 'ip', '-N', '-j', '-4', 'route', 'show', 'table', 'all').stdout)
        assert any(str(row.get('type')) in ('unreachable', '7') and row.get('dst') == mesh_prefix for row in rows), 'owned Mesh sink not retained'
    result['normal_tun']['http_outage_lease_sink'] = True
    spawn(controller, 'controller', ['/usr/local/bin/meshway-controller', 'serve', '--bind', '0.0.0.0:8420', '--data-dir', '/run/controller'])
    until('protected controller recovery and real TUN HTTP', traffic)
    result['normal_tun']['controller_recovery'] = True
    stop(agents['agent-a'], 'agent')
    until('clean Agent withdrawal', lambda: docker('exec', agents['agent-a'], 'ip', 'link', 'show', 'meshway0', check=False).returncode != 0, 20)
    lookup = docker('exec', agents['agent-a'], 'ip', '-4', 'route', 'get', addresses[b], check=False)
    assert lookup.returncode != 0
    spawn(agents['agent-a'], 'agent', ['/usr/local/bin/meshway-agent', 'run', '--state', '/run/agent.json', '--apply-dataplane',
        '--interval-seconds', '2', '--connection-lease-seconds', '20', '--dns-listen', '127.0.0.1:5353'])
    until('same identity Agent restart', traffic)
    assert {node['id'] for node in rpc(prefix + '/nodes')['nodes']} == {a, b}
    result['normal_tun']['same_identity_restart'] = True
    for role, cid in agents.items():
        (logs / (role + '.log')).write_text(docker('exec', cid, 'cat', '/run/agent.log').stdout)
        (logs / (role + '-rules.json')).write_text(docker('exec', cid, 'ip', '-N', '-j', '-4', 'rule', 'show').stdout)
        (logs / (role + '-routes.json')).write_text(docker('exec', cid, 'ip', '-N', '-j', '-4', 'route', 'show', 'table', 'all').stdout)


if __name__ == '__main__':
    def terminate(_signal, _frame):
        raise KeyboardInterrupt('termination requested; cleaning owned resources')
    signal.signal(signal.SIGTERM, terminate)
    parser = argparse.ArgumentParser()
    parser.add_argument('--check-only', action='store_true')
    parser.add_argument('--compile-only', action='store_true')
    parser.add_argument('--compiler', default='meshway-underlay-compiler:mw-underlay-431-60aab4705c43')
    parser.add_argument('--runtime', default='meshway-underlay-integration:mw-underlay-431-60aab4705c43')
    sys.exit(run(parser.parse_args()))
