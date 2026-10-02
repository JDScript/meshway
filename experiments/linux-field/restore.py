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


def check_capabilities(add, drop, role):
    # Only the spelling alias actually used by Docker is recognized. Unknown
    # capabilities remain unknown, and duplicates are never collapsed into a set.
    aliases = {'CAP_NET_ADMIN': 'NET_ADMIN'}
    values = [] if add is None else add
    if role not in ['agent', 'controller'] or not isinstance(values, list) or any(not isinstance(cap, str) for cap in values):
        raise ValueError('invalid container capabilities')
    normalized = [aliases.get(cap, cap) for cap in values]
    if normalized != (['NET_ADMIN'] if role == 'agent' else []) or drop != ['ALL']:
        raise ValueError('container capabilities mismatch')


def own_kernel_bridge_route(route, network):
    if not network or not re.fullmatch(r'[0-9a-f]{64}', network['Id']):
        return False
    if set(route) - {'type', 'dst', 'dev', 'table', 'protocol', 'scope', 'prefsrc', 'flags'}:
        return False
    subnet = ipaddress.IPv4Network(network['IPAM']['Config'][0]['Subnet'])
    gateway = network['IPAM']['Config'][0]['Gateway']
    if route.get('dev') != 'br-' + network['Id'][:12] or route.get('protocol') != 'kernel' or route.get('prefsrc') != gateway:
        return False
    if route.get('flags', []) not in [[], ['linkdown']]:
        return False
    prefix = ipaddress.IPv4Network(route['dst'], strict=False)
    kind = route.get('type', 'unicast')
    if kind == 'unicast':
        return prefix == subnet and route.get('scope') == 'link' and route.get('table', 'main') == 'main'
    if route.get('table') != 'local' or prefix.prefixlen != 32:
        return False
    return (kind == 'local' and str(prefix.network_address) == gateway and route.get('scope') == 'host') or (
        kind == 'broadcast' and prefix.network_address == subnet.broadcast_address and route.get('scope') == 'link')


def check_image_origin(view, image, source):
    if view['Id'] != image or (view['Config'].get('Labels') or {}).get('org.opencontainers.image.revision') != source:
        raise ValueError('local immutable image/source mismatch')


def check_runtime_constants(config, hc, role):
    for key, value in {'SSL_CERT_FILE': '/private/ca.pem', 'SSL_CERT_DIR': '/private/empty-certs'}.items():
        entries = [item for item in (config.get('Env') or []) if item.split('=', 1)[0] == key]
        if entries != [key + '=' + value]:
            raise ValueError('container SSL environment mismatch')
    sysctls = {'net.ipv4.ip_forward': '0', 'net.ipv6.conf.all.forwarding': '0',
               'net.ipv4.ping_group_range': '0 2147483647'}
    if role == 'agent':
        sysctls['net.ipv6.conf.default.disable_ipv6'] = '1'
    if hc.get('Sysctls') != sysctls or hc.get('PidsLimit') != 128 or hc.get('Memory') != 384 * 1024 * 1024 or hc.get('NanoCpus') != 1_000_000_000 or hc.get('Init') is not True:
        raise ValueError('container runtime constants mismatch')


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
                if not own_kernel_bridge_route(route, existing):
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
    check_image_origin(view, image, source)
    public_code = data['public_source'] + '\nprint(json.dumps(public_state(' + repr(role) + ')))'

    def public():
        return json.loads(dk(['run', '--pull', 'never', '--rm', '--name', name + '-' + role + '-verify',
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
        check_capabilities(hc['CapAdd'], hc['CapDrop'], role)
        check_runtime_constants(v['Config'], hc, role)
        if hc['Privileged'] or hc['NetworkMode'] != name:
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
            args = ['create', '--pull', 'never', '--name', container, '--init', '--user', '0:0', '--network', name, '--cap-drop', 'ALL',
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
