"""Explicit same-state restore. Loaded only by remote.py restore-* actions."""
import ipaddress
import json
import os
from pathlib import Path
import re
import subprocess

OWNER = 'io.meshway.field'
ATTEMPT = OWNER + '.attempt'
ROLE = OWNER + '.role'
SOURCE = OWNER + '.source'


def restore(data, root, docker):
    run_id, attempt = data['run_id'], data['attempt']
    if not re.fullmatch(r'[a-z0-9-]{10,48}', attempt) or attempt == run_id:
        raise ValueError('invalid distinct attempt')
    if root.parent.is_symlink() or root.is_symlink() or not root.is_dir() or root.stat().st_uid != os.getuid() or root.stat().st_mode & 0o077:
        raise ValueError('original private parent missing')
    marker = root / 'owner'
    if marker.is_symlink() or not marker.is_file() or marker.read_text() != run_id:
        raise ValueError('original owner marker missing')
    name = 'meshway-field-' + run_id + '-' + attempt

    def run(args, timeout=70, input=None):
        p = subprocess.run(args, input=input, capture_output=True, timeout=timeout)
        if p.returncode:
            raise RuntimeError('restore command failed; private child output suppressed')
        return p.stdout

    def dk(args, **kwargs):
        return run(docker + args, **kwargs)

    def inspect(kind, resource):
        # A missing resource is distinct from a failed Docker connection/permission.
        listing = [json.loads(line) for line in dk(([kind] if kind == 'network' else ['container']) + ['ls', *(['--all'] if kind == 'container' else []), '--format', 'json']).decode().splitlines()]
        matches = [v for v in listing if resource in [v.get('Name'), v.get('Names'), v.get('ID')]]
        if not matches:
            return None
        return json.loads(dk(([kind] if kind == 'network' else []) + ['inspect', resource]))[0]

    labels = {OWNER: run_id, ATTEMPT: attempt}

    def check_network(v):
        if v['Name'] != name or any(v['Labels'].get(k) != val for k, val in labels.items()):
            raise ValueError('unknown network conflict')
        if v['Driver'] != 'bridge' or v['IPAM']['Config'] != [{'Subnet': '172.31.249.0/28', 'Gateway': '172.31.249.1'}]:
            raise ValueError('network IPAM mismatch')

    action = data['action']
    if action == 'restore-cleanup-network':
        existing = inspect('network', name)
        if not existing:
            raise ValueError('cleanup network missing')
        check_network(existing)
        if existing['Containers']:
            raise ValueError('network still has attached containers')
        dk(['network', 'rm', name])
        return {'removed_network': name, 'private_state_retained': True}
    if action == 'restore-network':
        if inspect('network', 'meshway-field-' + run_id):
            raise ValueError('original bridge was not cleaned')
        existing = inspect('network', name)
        if existing:
            check_network(existing)
        routes = json.loads(run(['ip', '-j', '-4', 'route', 'show', 'table', 'all']))
        for route in routes:
            if route.get('dst', 'default') == 'default':
                continue
            prefix = ipaddress.IPv4Network(route['dst'], strict=False)
            if any(prefix.overlaps(ipaddress.IPv4Network(v)) for v in ['172.31.249.0/28', '198.18.42.0/24']):
                own_bridge = existing and route.get('dev') == 'br-' + existing['Id'][:12] and str(prefix) == '172.31.249.0/28'
                if not own_bridge:
                    raise ValueError('non-default route conflict')
        for route in json.loads(run(['ip', '-j', '-6', 'route', 'show', 'table', 'all'])):
            if route.get('dst', 'default') != 'default' and ipaddress.IPv6Network(route['dst'], strict=False).overlaps(ipaddress.IPv6Network('fd62:bfaa:4242::/64')):
                raise ValueError('non-default IPv6 route conflict')
        if not existing:
            dk(['network', 'create', '--driver', 'bridge', '--subnet', '172.31.249.0/28',
                '--label', OWNER + '=' + run_id, '--label', ATTEMPT + '=' + attempt, name])
        return {'network': name, 'resumed': bool(existing)}

    role = data['role']
    if role not in ['agent', 'controller']:
        raise ValueError('unknown role')
    directory = root / role
    # Do not traverse uid-0 role directories from SSH or copy their contents.
    removed = root / (role + '.removed-id')
    if removed.is_symlink() or not removed.is_file() or not re.fullmatch(r'[0-9a-f]{64}', removed.read_text().strip()):
        raise ValueError('original cleanup marker missing')
    if (root / (role + '.id')).exists() or inspect('container', 'meshway-field-' + run_id + '-' + role):
        raise ValueError('original runtime is not in the cleaned state')
    if directory.is_symlink() or not directory.is_dir() or directory.stat().st_uid != 0 or directory.stat().st_mode & 0o077:
        raise ValueError('original private uid-0 role directory missing')
    image, source = data['image'], data['source']
    if not re.fullmatch(r'sha256:[0-9a-f]{64}', image) or not re.fullmatch(r'[0-9a-f]{40}', source) or data['verified'] is not True:
        raise ValueError('verified immutable inventory required')
    view = json.loads(dk(['image', 'inspect', image]))[0]
    if view['Id'] != image:
        raise ValueError('local immutable image mismatch')
    public_code = data['public_source'] + '\nprint(json.dumps(public_state(' + repr(role) + ')))'

    def public():
        return json.loads(dk(['run', '--rm', '--name', name + '-' + role + '-verify',
                             '--label', OWNER + '=' + run_id, '--label', ATTEMPT + '=' + attempt,
                             '--label', ROLE + '=' + role, '--label', SOURCE + '=' + source,
                             '--user', '0:0', '--network', 'none', '--cap-drop', 'ALL', '--read-only',
                             '--security-opt', 'no-new-privileges', '--memory', '64m', '--cpus', '.25', '--pids-limit', '16',
                             '--mount', f'type=bind,source={directory},target=/private,readonly',
                             '--entrypoint', 'python3', image, '-I', '-c', public_code]))

    observed = public()
    if observed != data['expected_public']:
        raise ValueError('original seed/public identity/hash inventory mismatch')
    if action == 'restore-public':
        return observed
    container = name + '-' + role
    expected_labels = {**labels, ROLE: role, SOURCE: source}
    network = inspect('network', name)
    if not network:
        raise ValueError('restore network missing')
    check_network(network)

    def check_container(v):
        if v['Name'] != '/' + container or v['Image'] != image or any(v['Config']['Labels'].get(k) != val for k, val in expected_labels.items()):
            raise ValueError('unknown container conflict')
        hc = v['HostConfig']
        binds = [m for m in v['Mounts'] if m['Type'] == 'bind']
        if len(binds) != 1 or binds[0]['Source'] != str(directory) or binds[0]['Destination'] != '/private' or not binds[0]['RW']:
            raise ValueError('bind path mismatch')
        if any(m['Type'] != 'bind' and not (m['Type'] == 'tmpfs' and m['Destination'] == '/tmp') for m in v['Mounts']):
            raise ValueError('unknown additional mount')
        if hc['Privileged'] or hc['NetworkMode'] != name or hc['CapDrop'] != ['ALL'] or (hc['CapAdd'] or []) != (['NET_ADMIN'] if role == 'agent' else []):
            raise ValueError('container isolation mismatch')
        if not hc['ReadonlyRootfs'] or v['Config']['User'] != '0:0' or v['Config']['Entrypoint'] != ['python3'] or v['Config']['Cmd'] != ['-I', '/private/container.py', 'boot']:
            raise ValueError('container entrypoint mismatch')
        if 'no-new-privileges' not in (hc['SecurityOpt'] or []) or set(v['NetworkSettings']['Networks']) != {name}:
            raise ValueError('container security/network mismatch')
        expected_ports = {'18420/tcp': [{'HostIp': '0.0.0.0', 'HostPort': '28420'}]} if role == 'controller' else (
            {'18443/tcp': [{'HostIp': '0.0.0.0', 'HostPort': '28443'}], '18444/udp': [{'HostIp': '0.0.0.0', 'HostPort': '28444'}]} if data['listener'] else {})
        if (hc['PortBindings'] or {}) != expected_ports:
            raise ValueError('published port mismatch')
        if role == 'agent' and hc['Devices'] != [{'PathOnHost': '/dev/net/tun', 'PathInContainer': '/dev/net/tun', 'CgroupPermissions': 'rw'}]:
            raise ValueError('TUN device mismatch')
        if role == 'controller' and hc['Devices']:
            raise ValueError('unexpected Controller device')

    existing = inspect('container', container)
    if existing:
        check_container(existing)
        if existing['State']['Paused'] or existing['State']['Restarting']:
            raise ValueError('existing container is not safely resumable')
    if action == 'restore-container':
        if not existing:
            args = ['create', '--name', container, '--init', '--user', '0:0', '--network', name, '--cap-drop', 'ALL',
                    '--security-opt', 'no-new-privileges', '--read-only', '--memory', '384m', '--cpus', '1', '--pids-limit', '128',
                    '--mount', f'type=bind,source={directory},target=/private', '--tmpfs', '/tmp:rw,nosuid,nodev,noexec,size=16m',
                    '--env', 'SSL_CERT_FILE=/private/ca.pem', '--env', 'SSL_CERT_DIR=/private/empty-certs',
                    '--sysctl', 'net.ipv4.ip_forward=0', '--sysctl', 'net.ipv6.conf.all.forwarding=0',
                    '--sysctl', 'net.ipv4.ping_group_range=0 2147483647']
            for k, v in expected_labels.items():
                args += ['--label', k + '=' + v]
            if role == 'agent':
                args += ['--cap-add', 'NET_ADMIN', '--device', '/dev/net/tun:/dev/net/tun:rw', '--sysctl', 'net.ipv6.conf.default.disable_ipv6=1']
                if data['listener']:
                    args += ['--publish', '0.0.0.0:28443:18443/tcp', '--publish', '0.0.0.0:28444:18444/udp']
            else:
                args += ['--publish', '0.0.0.0:28420:18420/tcp']
            dk(args + ['--entrypoint', 'python3', image, '-I', '/private/container.py', 'boot'])
        if not existing or not existing['State']['Running']:
            dk(['start', container])
        return {'container': container, 'resumed': bool(existing), 'public': observed}
    if not existing:
        raise ValueError('attempt container missing')
    if action in ['restore-stop', 'restore-start', 'restore-restart']:
        verb = action.removeprefix('restore-')
        dk([verb, *(['--time', '20'] if verb != 'start' else []), container])
        return {'action': verb, 'container': container}
    if action == 'restore-helper':
        if data['helper'] not in ['probe', 'observe', 'dns']:
            raise ValueError('restore helper is read-only whitelist')
        return json.loads(dk(['exec', '-i', container, 'python3', '-I', '/private/container.py', data['helper']],
                             input=json.dumps(data.get('input', {})).encode()))
    if action == 'restore-cleanup':
        dk(['rm', '-f', container])
        return {'removed': role, 'private_state_retained': True}
    raise ValueError('unknown restore action')
