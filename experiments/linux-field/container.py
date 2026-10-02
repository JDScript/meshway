#!/usr/bin/env python3
"""Isolated field-test process supervisor and public packet evidence.

This is test infrastructure, not an alternative Agent configuration/control API.
The Agent enrolls and consumes the normal Controller publication unchanged.
"""
import hashlib
import datetime
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
import os
from pathlib import Path
import select
import signal
import socket
import socketserver
import ssl
import subprocess
import sys
import time

os.umask(0o077)
ROOT = Path('/private')
STATE = ROOT / 'agent.json'


def config():
    return json.loads((ROOT / 'field.json').read_text())


def checked(args, timeout=30):
    result = subprocess.run(args, capture_output=True, timeout=timeout)
    if result.returncode:
        # Local private log permits diagnosis without returning credentials.
        with (ROOT / 'helper-errors.log').open('ab') as output:
            output.write(result.stderr[-65536:])
        raise RuntimeError('command failed; private diagnostics retained')
    return result.stdout


def mesh_ip(value):
    address = ipaddress.IPv4Address(value)
    if address not in ipaddress.IPv4Network(config()['mesh_prefix']):
        raise ValueError('not a field network address')
    return str(address)


def serve():
    value = json.loads((ROOT / 'http.json').read_text())
    payload = value['body'].encode()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200 if self.path == '/meshway-field' else 404)
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *_):
            pass

    ThreadingHTTPServer((mesh_ip(value['address']), 43000), Handler).serve_forever()


def tls_proxy():
    # Fixed, loopback-only Controller upstream. TLS is verified by all clients
    # against the private test CA; no trust or route changes on the host.
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(ROOT / 'server.pem', ROOT / 'server.key')

    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            try:
                self.request.settimeout(10)
                with context.wrap_socket(self.request, server_side=True) as downstream:
                    with socket.create_connection(('127.0.0.1', 8420), timeout=10) as upstream:
                        while True:
                            readable = [downstream] if downstream.pending() else select.select([downstream, upstream], [], [], 15)[0]
                            if not readable:
                                return
                            for source in readable:
                                payload = source.recv(65536)
                                if not payload:
                                    return
                                (upstream if source is downstream else downstream).sendall(payload)
            except (OSError, ssl.SSLError):
                return

    class Server(socketserver.ThreadingTCPServer):
        allow_reuse_address = True
        daemon_threads = True

    with Server(('0.0.0.0', 18420), Handler) as server:
        server.serve_forever()


def boot():
    value = config()
    children = {}
    stopping = False

    def terminate(*_):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGTERM, terminate)
    signal.signal(signal.SIGINT, terminate)

    def start(name, command):
        with (ROOT / (name + '.log')).open('ab') as log:
            child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        children[name] = child
        (ROOT / (name + '.pid')).write_text(str(child.pid))

    try:
        if value['role'] == 'controller':
            start('controller', ['meshway-controller', 'serve', '--bind', '127.0.0.1:8420', '--data-dir', '/private/controller',
                                 '--ipv4-prefix', value['mesh_prefix'], '--ipv6-prefix', value['mesh_ipv6_prefix']])
            start('tls', ['python3', '-I', __file__, 'tls'])
        while not stopping:
            if value['role'] == 'agent' and 'agent' not in children and (ROOT / 'run.enabled').exists():
                start('agent', ['meshway-agent', 'run', '--state', str(STATE), '--apply-dataplane', '--tun-name', 'meshway0',
                                '--tun-mtu', '1000', '--interval-seconds', '3', '--connection-lease-seconds', '30',
                                '--dns-listen', '127.0.0.1:1053'])
            if (ROOT / 'http.json').exists() and 'http' not in children:
                address = json.loads((ROOT / 'http.json').read_text())['address']
                links = json.loads(checked(['ip', '-j', '-4', 'address', 'show']))
                if any(a['local'] == address for link in links for a in link['addr_info']):
                    start('http', ['python3', '-I', __file__, 'http'])
            if any(child.poll() is not None for child in children.values()):
                raise RuntimeError('field child exited; private logs retained')
            time.sleep(.2)
    finally:
        for child in children.values():
            if child.poll() is None:
                child.terminate()
        for child in children.values():
            try:
                child.wait(timeout=12)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()


def main(action, data):
    if action == 'admin-token':
        return {'token': checked(['meshway-controller', 'admin-token', '--data-dir', '/private/controller']).decode().strip()}
    if action == 'enroll':
        if STATE.exists():
            raise RuntimeError('refusing to overwrite node identity')
        value = config()
        checked(['meshway-agent', 'enroll', '--controller', value['controller'], '--name', value['name'],
                 '--state', str(STATE), '--token', data['token']])
        checked(['meshway-agent', 'connections', 'publish', '--state', str(STATE)])
        return {'node_id': json.loads(STATE.read_text())['node_id']}
    if action == 'enable':
        (ROOT / 'run.enabled').touch(mode=0o600)
        return {'enabled': True}
    if action == 'dns':
        address = mesh_ip(data['address'])
        prefix = ['meshway-agent', 'dns', 'query']
        options = ['--state', str(STATE), '--json']
        ptr = json.loads(checked(prefix + [address] + options))
        outcome = ptr['outcome']
        if outcome['status'] != 'ok' or outcome['result']['state'] != 'positive':
            raise RuntimeError('real PTR query failed')
        hostname = outcome['result']['answers'][0]['data']
        answers = []
        for tcp in [False, True]:
            value = json.loads(checked(prefix + [hostname, '--type', 'A'] + options + (['--tcp'] if tcp else [])))
            result = value['outcome']['result']
            if value['outcome']['status'] != 'ok' or result['state'] != 'positive' or not any(a['data'] == address for a in result['answers']):
                raise RuntimeError('real A query did not resolve intended Mesh node')
            answers.append(value)
        return {'ptr': ptr, 'a': answers}
    if action == 'http-start':
        address = mesh_ip(data['address'])
        body = data['body']
        if not isinstance(body, str) or not body.startswith('meshway-field:') or len(body) > 512:
            raise ValueError('invalid canary')
        (ROOT / 'http.json').write_text(json.dumps({'address': address, 'body': body}))
        return {'configured': True}
    if action == 'observe':
        value = {'observed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                 'addresses': json.loads(checked(['ip', '-j', 'address', 'show'])),
                 'routes': json.loads(checked(['ip', '-j', '-4', 'route', 'show', 'table', 'all'])),
                 'rules': json.loads(checked(['ip', '-j', '-4', 'rule', 'show'])),
                 'sockets': checked(['ss', '-H', '-ntup']).decode(), 'state_saved': STATE.exists(),
                 'state_private': STATE.exists() and STATE.stat().st_mode & 0o777 == 0o600,
                 'state_sha256': hashlib.sha256(STATE.read_bytes()).hexdigest() if STATE.exists() else None,
                 'material_saved': (ROOT / 'agent.json.transport-materials.json').exists(),
                 'legacy_profiles_absent': not (ROOT / 'profiles.json').exists()}
        if STATE.exists():
            value['node_id'] = json.loads(STATE.read_text())['node_id']
        log = ROOT / 'agent.log'
        value['dataplane'] = None
        if log.exists() and log.stat().st_size <= 16 * 1024 * 1024:
            for line in log.read_text().splitlines():
                try:
                    event = json.loads(line)
                except ValueError:
                    continue
                if isinstance(event, dict) and event.get('event') == 'managed_dataplane':
                    value['dataplane'] = {k: event.get(k) for k in ['node_id', 'revision', 'digest', 'tun_name', 'tun_ready', 'allowed_paths', 'route_sink_retained', 'reason']}
        return value
    if action == 'probe':
        address = mesh_ip(data['address'])
        ping = subprocess.run(['ping', '-4', '-n', '-c', '3', '-W', '2', '-w', '8', address], capture_output=True, text=True, timeout=10)
        connection = http.client.HTTPConnection(address, 43000, timeout=5)
        started = time.monotonic()
        http_ok = False
        matches = False
        try:
            connection.request('GET', '/meshway-field', headers={'Connection': 'close'})
            response = connection.getresponse()
            http_ok = response.status == 200
            matches = response.read(4097) == data['body'].encode()
        except (OSError, http.client.HTTPException):
            pass
        finally:
            connection.close()
        route = subprocess.run(['ip', '-j', '-4', 'route', 'get', address], capture_output=True, text=True)
        return {'ping_exit': ping.returncode, 'ping': ping.stdout[-2048:], 'http_ok': http_ok, 'body_matches': matches,
                'http_elapsed_ms': round((time.monotonic() - started) * 1000, 2),
                'route': json.loads(route.stdout) if route.returncode == 0 else None}
    raise ValueError('unknown helper action')


if __name__ == '__main__':
    try:
        action = sys.argv[1]
        if action == 'boot':
            boot()
        elif action == 'http':
            serve()
        elif action == 'tls':
            tls_proxy()
        else:
            print(json.dumps(main(action, json.load(sys.stdin))))
    except Exception:
        print('field helper failed; private diagnostics retained', file=sys.stderr)
        sys.exit(1)
