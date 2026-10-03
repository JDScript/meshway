"""Real TUN through automatic relay adjacency and a controllable UDP loss gate."""
import json
from pathlib import Path
import time


def until(description, function, seconds=90):
    deadline, last = time.monotonic() + seconds, None
    while time.monotonic() < deadline:
        try:
            value = function()
            if value:
                return value
        except Exception as error:
            last = str(error)
        time.sleep(.5)
    raise RuntimeError(description + ': ' + str(last))


def accept(docker, container, name, logs, result, networks):
    network = name + '-underlay'
    docker('network', 'create', '--internal', '--label', 'meshway.automatic.acceptance=' + name, network)
    networks.append(network)
    controller = container('controller', network, agent=False)
    agents = {role: container(role, network) for role in ('a', 'b', 'relay')}

    def spawn(cid, label, argv):
        code = ('import json,os,pathlib,subprocess,sys; os.umask(0o077); a=json.load(sys.stdin); '
            'p=subprocess.Popen(a,stdin=subprocess.DEVNULL,stdout=open("/run/' + label + '.log","ab"),stderr=subprocess.STDOUT); '
            'pathlib.Path("/run/' + label + '.pid").write_text(str(p.pid))')
        docker('exec', '-i', cid, 'python3', '-c', code, input=json.dumps(argv))

    spawn(controller, 'controller', ['meshway-controller', 'serve', '--bind', '0.0.0.0:8420', '--data-dir', '/run/controller'])
    admin = until('controller initialization', lambda: docker('exec', controller, 'meshway-controller',
        'admin-token', '--data-dir', '/run/controller').stdout.strip(), 20)
    rpc_code = ('import http.client,json,sys; d=json.load(sys.stdin); c=http.client.HTTPConnection("127.0.0.1",8420,timeout=5); '
        'b=None if d["body"] is None else json.dumps(d["body"]); '
        'c.request("GET" if b is None else "POST",d["path"],b,{"Authorization":"Bearer "+d["token"],"Content-Type":"application/json"}); '
        'r=c.getresponse(); s=r.read(); assert r.status==200,(r.status,s.decode()); print(s.decode())')

    def rpc(path, body=None):
        return json.loads(docker('exec', '-i', controller, 'python3', '-c', rpc_code,
            input=json.dumps({'token': admin, 'path': path, 'body': body})).stdout)

    scope = until('controller readiness', lambda: rpc('/api/v1/networks'), 20)['networks'][0]['network_id']
    prefix = '/api/v1/networks/' + scope
    nodes = {}
    command = ['meshway-agent', 'run', '--state', '/run/agent.json', '--apply-dataplane',
               '--interval-seconds', '2', '--connection-lease-seconds', '20']
    for role, cid in agents.items():
        token = rpc(prefix + '/enrollment-tokens', {'expires_in_seconds': 300})['token']
        enrolled = docker('exec', cid, 'meshway-agent', 'enroll', '--controller', 'http://controller.mesh.test:8420',
            '--name', role, '--token', token, '--state', '/run/agent.json')
        (logs / (role + '-enroll.log')).write_text(enrolled.stdout + enrolled.stderr)
        status = json.loads(docker('exec', cid, 'meshway-agent', 'status', '--state', '/run/agent.json').stdout)
        nodes[role] = status['node_id']
        spawn(cid, 'agent', command)
    inventory = until('normal identity publication', lambda: (lambda s: s if all(any(n['node']['id'] == node and n['public_key']
        for n in s['nodes']) for node in nodes.values()) else False)(rpc(prefix + '/connections')))
    addresses = {entry['node']['id']: entry['node']['mesh_ipv4'] for entry in inventory['nodes']}
    policy = rpc(prefix + '/policy')['policy']
    policy['grants'] = [{'id': 'endpoints', 'src': ['node:' + nodes['a']], 'dst': ['node:' + nodes['b']], 'bidirectional': True}]

    def save_policy():
        return rpc(prefix + '/policy/save', {'expected_revision': rpc(prefix + '/policy')['revision'],
            'policy': policy, 'source_jsonc': None})

    save_policy()
    gate = Path(__file__).with_name('udp_gate.py').read_text()
    docker('exec', '-i', controller, 'python3', '-c', 'import sys,pathlib; pathlib.Path("/run/udp_gate.py").write_text(sys.stdin.read())', input=gate)
    spawn(controller, 'udp-gate', ['python3', '/run/udp_gate.py', 'relay.mesh.test', '43101'])
    for transport, port, host in [('native_udp', 43101, 'controller.mesh.test'), ('native_tcp', 43102, 'relay.mesh.test')]:
        rpc(prefix + '/managed-listeners', {'expected_revision': rpc(prefix + '/connections')['revision'], 'settings': {
            'id': 'relay-' + transport, 'node_id': nodes['relay'], 'enabled': True, 'transport': transport,
            'bind_address': '0.0.0.0', 'bind_port': port,
            'advertised_endpoints': [{'host': host, 'port': port}], 'vless': None}})
    assert rpc(prefix + '/connections')['configuration']['candidates'] == []
    revision = None
    expected_paths = sorted([[nodes['a'], nodes['relay'], nodes['b']], [nodes['b'], nodes['relay'], nodes['a']]])
    snapshot_code = ('import json,pathlib; s=json.loads(pathlib.Path("/run/agent.json.mesh-execution.json").read_text())["snapshot"]; '
                     'p=s["payload"]; print(json.dumps({k:p[k] for k in ("revision","network_id","node_id","paths","connections")}))')

    def coherent():
        nonlocal revision
        current = rpc(prefix + '/policy')['revision']
        if revision is not None and current != revision:
            return False
        observed = {role: json.loads(docker('exec', cid, 'python3', '-c', snapshot_code).stdout) for role, cid in agents.items()}
        (logs / 'last-execution.json').write_text(json.dumps(observed, indent=2))
        for role, value in observed.items():
            if value['revision'] != current or value['network_id'] != scope or sorted(p['nodes'] for p in value['paths']) != expected_paths:
                return False
            connections = value['connections']
            if role != 'relay':
                assert not connections['listeners'] and len(connections['outgoing']) == 2
                assert all(c['id'].startswith('auto.') and c['peer_node_id'] == nodes['relay'] for c in connections['outgoing'])
        result['coherent_execution'] = observed
        revision = current
        return True

    until('automatic same-revision relay paths', coherent)
    for role, cid in agents.items():
        ip = addresses[nodes[role]]
        until(role + ' TUN assignment', lambda cid=cid, ip=ip: ip in docker('exec', cid, 'ip', '-j', '-4', 'address', 'show', 'dev', 'meshway0').stdout)
        docker('exec', cid, 'mkdir', '/run/business')
        spawn(cid, 'business', ['python3', '-m', 'http.server', '8088', '--bind', ip, '--directory', '/run/business'])
    probe = ('import http.client,sys; c=http.client.HTTPConnection(sys.argv[1],8088,timeout=2); '
             'c.request("GET","/"); r=c.getresponse(); assert r.status==200; r.read(); print("real-tun-http")')

    def traffic():
        return all(docker('exec', agents[source], 'python3', '-c', probe, addresses[nodes[target]], check=False, timeout=6).returncode == 0
                   for source, target in [('a', 'b'), ('b', 'a')])

    def transport(kind, label):
        snapshot = rpc(prefix + '/connections/runtime')
        for role in ('a', 'b'):
            value = next(n for n in snapshot['nodes'] if n['node_id'] == nodes[role])
            if not value['current'] or value['stale'] or value['revision'] != revision:
                return False
            if not any(p['peer_node_id'] == nodes['relay'] and p['transport'] == kind and p['state'] == 'authenticated'
                       and p['probe_fresh'] and p['probe_rtt_ms'] is not None for p in value['peers']):
                return False
        if not traffic():
            return False
        result[label] = snapshot
        return True

    until('authenticated UDP relay TUN traffic', lambda: transport('native_udp', 'udp_primary'))
    docker('exec', controller, 'touch', '/run/drop-udp')
    until('UDP loss selects TCP on unchanged logical adjacency', lambda: transport('native_tcp', 'tcp_fallback'))
    assert rpc(prefix + '/policy')['revision'] == revision
    docker('exec', controller, 'rm', '/run/drop-udp')
    until('UDP recovery preempts TCP on unchanged revision', lambda: transport('native_udp', 'udp_recovered'))
    assert coherent() and rpc(prefix + '/policy')['revision'] == revision
    result['coverage'] = {'two_outbound_only_endpoints': True, 'source_candidates': 0, 'same_revision_udp_tcp_udp': True,
                          'real_bidirectional_tun_http': True, 'nat_punch': False,
                          'udp_loss_fixture': 'transparent userspace UDP gate; no NAT or direct-connect claim'}
    grants = policy['grants']
    policy['grants'] = []
    withdrawn_revision = save_policy()['revision']

    def withdrawn():
        observed = {role: json.loads(docker('exec', cid, 'python3', '-c', snapshot_code).stdout) for role, cid in agents.items()}
        if not all(p['revision'] == withdrawn_revision and not p['paths'] and not p['connections']['outgoing']
                   and not p['connections']['incoming'] for p in observed.values()):
            return False
        result['withdrawn_execution'] = observed
        return True

    until('grant withdrawal removes automatic adjacency and packet paths', withdrawn)
    assert all(docker('exec', agents[source], 'python3', '-c', probe, addresses[nodes[target]], check=False, timeout=6).returncode != 0
               for source, target in [('a', 'b'), ('b', 'a')]), 'withdrawn grant still permits TUN HTTP'
    policy['grants'] = grants
    save_policy()
    revision = None
    until('grant restoration republishes coherent automatic paths', coherent)
    until('grant restoration recovers real TUN HTTP', lambda: transport('native_udp', 'grant_restored'))
    result['coverage']['grant_withdrawal_and_restoration'] = True
