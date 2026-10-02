#!/usr/bin/env python3
"""SSH-side Docker-only field deployment; never edits host network settings."""
import base64
import ipaddress
import json
import os
from pathlib import Path
import re
import subprocess
import sys

os.umask(0o077)
OWNER = 'io.meshway.field'


def main(data):
    run_id = data['run_id']
    if not re.fullmatch(r'[a-z0-9-]{10,48}', run_id):
        raise ValueError('invalid run id')
    root = Path.home() / '.meshway-field' / run_id
    name = 'meshway-field-' + run_id
    docker = ['/usr/bin/docker', '--host', 'unix:///var/run/docker.sock']
    if data['sudo']:
        docker = ['sudo', '-n', '--'] + docker

    def run(args, timeout=60, input=None):
        result = subprocess.run(args, input=input, capture_output=True, timeout=timeout)
        if result.returncode:
            if root.exists():
                (root / 'last-command-error.log').write_bytes(result.stderr[-65536:])
            raise RuntimeError('remote command failed; details retained privately')
        return result.stdout

    def dk(args, **kwargs):
        return run(docker + args, **kwargs)

    def owned(kind, resource):
        value = dk(([kind] if kind == 'network' else []) + ['inspect', resource, '--format',
                     '{{index ' + ('.Labels' if kind == 'network' else '.Config.Labels') + ' "' + OWNER + '"}}'])
        if value.decode().strip() != run_id:
            raise RuntimeError('resource owner mismatch')

    action = data['action']
    if action == 'initialize':
        if root.parent.is_symlink():
            raise ValueError('deployment parent must not be a symlink')
        root.parent.mkdir(mode=0o700, exist_ok=True)
        if root.parent.stat().st_uid != os.getuid() or root.parent.stat().st_mode & 0o077:
            raise ValueError('deployment parent must be private and owned')
        routes = json.loads(run(['ip', '-j', '-4', 'route', 'show', 'table', 'all']))
        reserved = [ipaddress.IPv4Network('172.31.249.0/28'), ipaddress.IPv4Network('198.18.42.0/24')]
        for route in routes:
            destination = route.get('dst', 'default')
            if destination == 'default':
                continue
            network = ipaddress.IPv4Network(destination, strict=False)
            if any(network.overlaps(test) for test in reserved):
                raise ValueError('field subnet overlaps an existing host route')
        root.mkdir(mode=0o700, parents=True, exist_ok=False)
        (root / 'owner').write_text(run_id)
        # Preselected subnet is only added through Docker's isolated bridge.
        dk(['network', 'create', '--driver', 'bridge', '--subnet', '172.31.249.0/28', '--label', OWNER + '=' + run_id, name])
        return {'root': str(root), 'network': name}
    if not root.is_dir() or (root / 'owner').read_text() != run_id:
        raise RuntimeError('private deployment owner mismatch')
    if action == 'create':
        role = data['role']
        if role not in ['agent', 'controller']:
            raise ValueError('invalid role')
        image = data['image']
        if not re.fullmatch(r'sha256:[0-9a-f]{64}', image):
            raise ValueError('immutable image required')
        owned('network', name)
        directory = root / role
        directory.mkdir(mode=0o700)
        for filename, encoded in data['files'].items():
            if filename not in ['field.json', 'container.py', 'ca.pem', 'server.pem', 'server.key']:
                raise ValueError('invalid deployment file')
            (directory / filename).write_bytes(base64.b64decode(encoded, validate=True))
        container = name + '-' + role
        args = ['create', '--name', container, '--init', '--label', OWNER + '=' + run_id,
                '--network', name, '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                '--read-only', '--mount', f'type=bind,source={directory},target=/private',
                '--tmpfs', '/tmp:rw,nosuid,nodev,noexec,size=16m', '--memory', '384m', '--cpus', '1', '--pids-limit', '128',
                '--env', 'SSL_CERT_FILE=/private/ca.pem', '--env', 'SSL_CERT_DIR=/private/empty-certs',
                '--sysctl', 'net.ipv4.ip_forward=0', '--sysctl', 'net.ipv6.conf.all.forwarding=0',
                '--sysctl', 'net.ipv4.ping_group_range=0 2147483647']
        if role == 'agent':
            args += ['--cap-add', 'NET_ADMIN', '--device', '/dev/net/tun:/dev/net/tun:rw',
                     '--sysctl', 'net.ipv6.conf.default.disable_ipv6=1']
            if data.get('listener'):
                args += ['--publish', '0.0.0.0:28443:18443/tcp', '--publish', '0.0.0.0:28444:18444/udp']
        else:
            args += ['--publish', '0.0.0.0:28420:18420/tcp']
        args += ['--entrypoint', 'python3', image, '-I', '/private/container.py', 'boot']
        container_id = dk(args).decode().strip()
        (root / (role + '.id')).write_text(container_id)
        dk(['start', container])
        return {'id': container_id, 'name': container, 'directory': str(directory), 'image': image}
    if action == 'cleanup':
        removed = []
        errors = []
        for role in ['agent', 'controller']:
            marker = root / (role + '.id')
            if not marker.exists():
                continue
            container = marker.read_text()
            try:
                owned('container', container)
                dk(['rm', '-f', container])
                marker.rename(root / (role + '.removed-id'))
                removed.append(role)
            except Exception:
                errors.append(role)
        if not errors:
            owned('network', name)
            dk(['network', 'rm', name])
        return {'removed': removed, 'errors': errors, 'private_state_retained': str(root)}
    role = data['role']
    if role not in ['agent', 'controller']:
        raise ValueError('invalid role')
    container = (root / (role + '.id')).read_text()
    owned('container', container)
    if action == 'helper':
        return json.loads(dk(['exec', '-i', container, 'python3', '-I', '/private/container.py', data['helper']],
                             input=json.dumps(data.get('input', {})).encode(), timeout=70))
    if action in ['stop', 'start', 'restart']:
        dk([action, *(['--time', '20'] if action in ['stop', 'restart'] else []), container], timeout=70)
        return {'action': action, 'id': container}
    if action == 'inspect':
        view = json.loads(dk(['inspect', container]))[0]
        return {'id': view['Id'], 'state': view['State'], 'image': view['Image'],
                'network_mode': view['HostConfig']['NetworkMode'], 'privileged': view['HostConfig']['Privileged'],
                'cap_add': view['HostConfig']['CapAdd'], 'cap_drop': view['HostConfig']['CapDrop'],
                'ports': view['NetworkSettings']['Ports']}
    raise ValueError('unknown remote action')


if __name__ == '__main__':
    try:
        print(json.dumps(main(json.load(sys.stdin))))
    except Exception as error:
        # No argv, input JSON, secret state, or underlying child output.
        print('field remote failure: ' + type(error).__name__, file=sys.stderr)
        sys.exit(1)
