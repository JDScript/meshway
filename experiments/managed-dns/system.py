#!/usr/bin/env python3
"""System resolver acceptance helper; installed only in new, owned containers."""
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time

sys.dont_write_bytecode = True
import container as base
from wire import query

ROOT = Path('/private')
RESOLV = Path('/etc/resolv.conf')
ADAPTER = ROOT / 'resolver-code' / 'resolver.py'
os.umask(0o077)


def child(args, timeout=8):
    result = subprocess.run(args, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=timeout)
    return result


def resolver_args(action):
    return ['python3', str(ADAPTER), action, '--state-dir', '/private/resolver',
            *(['--resolv-conf', str(RESOLV), '--allow-container-resolver'] if action != 'status' else [])]


def apply_args(data):
    return [*resolver_args('run'), '--zone', data['zone'],
            '--mesh-ipv4-prefix', str(ipaddress.IPv4Network(data['ipv4_prefix'])),
            '--mesh-ipv6-prefix', str(ipaddress.IPv6Network(data['ipv6_prefix'])),
            '--listen', '127.0.0.1:53', '--nameservice', '127.0.0.1:5353']


def main(action, data):
    if action == 'resolver-start':
        return base.start('resolver', apply_args(data))
    if action == 'readonly-apply':
        before = RESOLV.read_bytes()
        result = child(apply_args(data))
        if result.returncode == 0:
            raise ValueError('read-only apply unexpectedly succeeded')
        for kind in (socket.SOCK_DGRAM, socket.SOCK_STREAM):
            with socket.socket(socket.AF_INET, kind) as listener:
                listener.bind(('127.0.0.1', 53))
        return {'rejected': True, 'unchanged': RESOLV.read_bytes() == before, 'listeners_released': True}
    if action == 'occupied-apply':
        before = RESOLV.read_bytes()
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as occupied:
            occupied.bind(('127.0.0.1', 53))
            occupied.listen(1)
            result = child(apply_args(data))
            if result.returncode == 0:
                raise ValueError('occupied port apply unexpectedly succeeded')
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as released:
                released.bind(('127.0.0.1', 53))
        return {'rejected': True, 'unchanged': RESOLV.read_bytes() == before, 'udp_released': True}
    if action == 'resolver-stop':
        return base.stop('resolver')
    if action == 'resolver-kill':
        pid = base.pid('resolver')
        if pid is None:
            raise ValueError('resolver must be live')
        os.kill(pid, signal.SIGKILL)
        for _ in range(40):
            if base.pid('resolver') is None:
                return {'stopped': True}
            time.sleep(.05)
        raise ValueError('resolver did not exit')
    if action == 'resolver-down':
        result = child(resolver_args('down'))
        return {'exit': result.returncode, 'digest': hashlib.sha256(RESOLV.read_bytes()).hexdigest()}
    if action == 'resolver-status':
        result = child(resolver_args('status'))
        if result.returncode:
            raise ValueError('resolver status failed')
        value = json.loads(result.stdout)
        return {key: value[key] for key in ('state', 'owner_lock', 'pid_alive', 'listener_udp', 'listener_tcp', 'resolver_owned')}
    if action == 'resolver-observe':
        lines = RESOLV.read_text().splitlines()
        return {'pid': base.pid('resolver'), 'digest': hashlib.sha256(RESOLV.read_bytes()).hexdigest(),
                'nameservers': [line.split()[1] for line in lines if line.startswith('nameserver ')],
                'search': [word for line in lines if line.startswith('search ') for word in line.split()[1:]]}
    if action == 'query':
        return query(data['name'], data['kind'], data['transport'], port=53)
    if action == 'getent':
        name = data['name']
        if not isinstance(name, str) or name.startswith('-') or len(name) > 255:
            raise ValueError('invalid name')
        result = child(['getent', 'ahostsv4', name], timeout=12)
        addresses = sorted({str(ipaddress.IPv4Address(line.split()[0])) for line in result.stdout.splitlines()})
        return {'exit': result.returncode, 'addresses': addresses}
    if action == 'http-start':
        address = str(ipaddress.IPv4Address(data['address']))
        base.atomic(ROOT / 'resolver-http.json', {'address': address, 'body': data['body']})
        return base.start('resolver-http', ['python3', __file__, 'serve'])
    if action == 'curl':
        name = data['name']
        if not isinstance(name, str) or any(c not in 'abcdefghijklmnopqrstuvwxyz0123456789.-' for c in name):
            raise ValueError('invalid curl hostname')
        result = child(['curl', '-4', '--noproxy', '*', '--silent', '--fail', '--connect-timeout', '2',
                        '--max-time', '3', '--write-out', '\n%{remote_ip}', f'http://{name}:43000/meshway-resolver'])
        body, _, address = result.stdout.rpartition('\n')
        return {'exit': result.returncode, 'body_matches': body == data['body'],
                'remote_ip': str(ipaddress.IPv4Address(address)) if address else None}
    if action == 'neighbor':
        peer = data['node_id']
        if not isinstance(peer, str) or len(peer) != 64 or any(c not in '0123456789abcdef' for c in peer):
            raise ValueError('invalid node identity')
        result = child(['meshway-agent', 'ping', peer, '--state', str(base.STATE), '--count', '1', '--timeout-seconds', '2'])
        return {'authenticated_probe': result.returncode == 0}
    if action == 'route':
        address = str(ipaddress.IPv4Address(data['address']))
        result = child(['ip', '-j', '-4', 'route', 'get', address])
        if result.returncode == 0:
            routes = json.loads(result.stdout)
            return {'kind': 'tun' if len(routes) == 1 and routes[0].get('dev') == 'meshway0' else 'underlay'}
        prefix = ipaddress.IPv4Network(data['prefix'])
        routes = json.loads(base.checked(['ip', '-j', '-4', 'route', 'show', 'table', 'all']))
        guards = [r for r in routes if r.get('dst') == str(prefix) and r.get('type') in ('blackhole', 'unreachable', 'prohibit')]
        return {'kind': 'guard' if guards else 'missing'}
    if action == 'conflict-create':
        # This fixture owns both versions. Keep exact copies to undo only its own test edit.
        current = RESOLV.read_bytes()
        (ROOT / 'resolver-owned.before').write_bytes(current)
        changed = current + b'\n# isolated external-owner conflict test\n'
        (ROOT / 'resolver-owned.conflict').write_bytes(changed)
        RESOLV.write_bytes(changed)
        return {'digest': hashlib.sha256(changed).hexdigest()}
    if action == 'conflict-undo':
        if RESOLV.read_bytes() != (ROOT / 'resolver-owned.conflict').read_bytes():
            raise ValueError('fixture no longer owns conflict edit')
        RESOLV.write_bytes((ROOT / 'resolver-owned.before').read_bytes())
        return {'restored_fixture_edit': True}
    raise ValueError('unknown system resolver action')


def serve():
    config = json.loads((ROOT / 'resolver-http.json').read_text())
    payload = config['body'].encode()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200 if self.path == '/meshway-resolver' else 404)
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *_args):
            pass

    ThreadingHTTPServer((config['address'], 43000), Handler).serve_forever()


if __name__ == '__main__':
    try:
        if sys.argv[1] == 'serve':
            serve()
        else:
            print(json.dumps(main(sys.argv[1], json.load(sys.stdin))))
    except Exception:
        print('system resolver helper failed; private diagnostics suppressed', file=sys.stderr)
        sys.exit(1)
