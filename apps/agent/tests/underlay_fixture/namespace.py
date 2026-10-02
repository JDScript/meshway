"""Disposable namespace only: production Rust factory is the client under test."""
import json
import os
from pathlib import Path
import signal
import socket
import ssl
import struct
import subprocess
import sys
import threading
from urllib.parse import urlparse, parse_qs

sys.path.insert(0, '/lab')
from fixture import TunSink, ip, dns_reply, SERVER


def record(**event):
    with open('/run/observer.jsonl', 'a') as output:
        output.write(json.dumps(event) + '\n')


def exact(conn, length):
    data = b''
    while len(data) < length:
        chunk = conn.recv(length - len(data))
        if not chunk:
            raise EOFError()
        data += chunk
    return data


def dns(request, tcp=False):
    end, labels = 12, []
    while request[end]:
        n = request[end]
        labels.append(request[end + 1:end + 1 + n].decode())
        end += n + 1
    end += 1
    hostname = '.'.join(labels)
    kind = struct.unpack('!H', request[end:end + 2])[0]
    record(service='dns', hostname=hostname, transport='tcp' if tcp else 'udp', kind=kind)
    if hostname.startswith('silent.'):
        return None
    flags = 0x8183 if hostname.startswith('missing.') else 0x8180
    if hostname.startswith('tcp.') and not tcp:
        flags |= 0x0200
    empty = hostname.startswith('missing.') or hostname.startswith('v4.') and kind == 28 or hostname.startswith('v6.') and kind == 1 or flags & 0x0200
    if empty:
        return request[:2] + struct.pack('!HHHHH', flags, 1, 0, 0, 0) + request[12:end + 4]
    reply = bytearray(dns_reply(request))
    # TTL=1 proves cache expiry against actual new observer queries.
    reply[end + 4 + 6:end + 4 + 10] = struct.pack('!I', 1)
    return bytes(reply)


def server():
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain('/fixture.crt', '/fixture.key')

    def udp(version, port):
        sock = socket.socket(socket.AF_INET if version == 4 else socket.AF_INET6, socket.SOCK_DGRAM)
        sock.bind((SERVER[version], port))
        while True:
            request, peer = sock.recvfrom(65535)
            record(service='udp-receive', port=port, source=peer[0], source_port=peer[1], version=version, payload=request.hex())
            reply = dns(request) if port == 41864 else request
            if reply is not None:
                sock.sendto(reply, peer)

    def handle(conn, peer, version, port):
        try:
            conn.settimeout(15)
            if port == 41862:
                conn = context.wrap_socket(conn, server_side=True)
            record(service='accept', port=port, source=peer[0], source_port=peer[1], version=version)
            if port == 41864:
                request = exact(conn, struct.unpack('!H', exact(conn, 2))[0])
                reply = dns(request, tcp=True)
                if reply is not None:
                    conn.sendall(struct.pack('!H', len(reply)) + reply)
                return
            request = b''
            while b'\r\n\r\n' not in request:
                request += exact(conn, 1)
            path = request.split(b' ', 2)[1].decode()
            record(service='http-request', path=path, version=version, source=peer[0])
            if path == '/headers':
                record(service='retirement-eof', path=path, eof=conn.recv(1) == b'')
                return
            if path == '/slow':
                conn.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx')
                record(service='retirement-eof', path=path, eof=conn.recv(1) == b'')
                return
            if path.startswith('/api/'):
                assert b'authorization: Bearer fixture-only'.lower() in request.lower()
                conn.sendall(b'HTTP/1.1 401 Unauthorized\r\nContent-Length: 100000\r\n\r\n')
                # Generated code must drop the body rather than wait for it.
                record(service='status-first-eof', eof=conn.recv(65535) == b'')
                return
            if path.startswith('/dial?'):
                params = parse_qs(urlparse(path).query)
                with socket.create_connection((params['address'][0], int(params['port'][0])), 3) as outgoing:
                    payload = b''
                    while len(payload) < 14:
                        payload += exact(outgoing, 1)
                    record(service='listener-synack', version=version, payload=payload.decode())
            body = json.dumps({'source': peer[0], 'version': version}).encode()
            conn.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: ' + str(len(body)).encode() + b'\r\nConnection: close\r\n\r\n' + body)
        except (OSError, EOFError, ValueError) as error:
            record(service='server-error', error=str(error), port=port)
        finally:
            conn.close()

    def tcp(version, port):
        sock = socket.socket(socket.AF_INET if version == 4 else socket.AF_INET6, socket.SOCK_STREAM)
        sock.bind((SERVER[version], port))
        sock.listen()
        while True:
            conn, peer = sock.accept()
            threading.Thread(target=handle, args=(conn, peer, version, port), daemon=True).start()

    for version in (4, 6):
        for port in (41864, 41865):
            threading.Thread(target=udp, args=(version, port), daemon=True).start()
        for port in (41861, 41862, 41864):
            threading.Thread(target=tcp, args=(version, port), daemon=True).start()
    Path('/run/ready').touch()
    signal.pause()


def client():
    before = {v: json.loads(ip(v, '-j', 'rule', 'show').stdout) for v in (4, 6)}
    sinks = [TunSink('mwprod4'), TunSink('mwprod6')]
    env = {**os.environ, 'MESHWAY_UNDERLAY_FIXTURE': '1'}
    result = {'status': 'failed', 'cleanup_errors': []}
    owned = []
    try:
        for v in (4, 6):
            src = '172.30.247.11/24' if v == 4 else 'fd74:6d77:247::11/64'
            ip(v, 'address', 'add', src, 'dev', 'eth0', *(['nodad'] if v == 6 else []))
            # An original VPN policy after the Meshway block must still run.
            prefix = '172.30.247.0/24' if v == 4 else 'fd74:6d77:247::/64'
            ip(v, 'route', 'add', prefix, 'dev', 'eth0', 'src', src.split('/')[0], 'table', '18550')
            ip(v, 'rule', 'add', 'priority', '19000', 'fwmark', '0x20/0xff', 'lookup', '18550')
            owned.append((v, 'vpn'))
            ip(v, 'rule', 'add', 'priority', '18003', 'nop')
            owned.append((v, 'nop'))
            ip(v, 'rule', 'add', 'priority', '18000', 'fwmark', '0x4d570000/0xffff0000', 'iif', 'lo', 'goto', '18003')
            owned.append((v, 'guard'))
            ip(v, 'route', 'add', 'default', 'dev', sinks[0 if v == 4 else 1].name, 'table', '18401')
            ip(v, 'rule', 'add', 'priority', '18001', 'iif', 'lo', 'lookup', '18401')
            owned.append((v, 'capture'))
        # Namespace-only rt_tables display alias. The inactive reserve must be
        # checked even though priority 18002 has deliberately not been installed.
        Path('/etc/iproute2/rt_tables').write_text('255 local\n254 main\n253 default\n0 unspec\n18402 mw_capture_two\n')
        result['ur1_alias_checks'] = []
        for v in (4, 6):
            ip(v, 'route', 'add', 'default', 'dev', sinks[0 if v == 4 else 1].name, 'table', '18402')
            try:
                ip(v, 'rule', 'add', 'priority', '19001', 'lookup', '18402')
                try:
                    plain = json.loads(ip(v, '-j', 'rule', 'show').stdout)
                    numeric = json.loads(ip(v, '-N', '-j', 'rule', 'show').stdout)
                    assert not any(r['priority'] == 18002 for r in plain)
                    assert next(r for r in plain if r['priority'] == 19001)['table'] == 'mw_capture_two'
                    assert int(next(r for r in numeric if r['priority'] == 19001)['table']) == 18402
                    checked = subprocess.run(['/underlay-tests', '--exact', 'underlay::fixture::alias_capture_reference_rejected', '--ignored', '--nocapture'], env=env, capture_output=True, text=True, timeout=10)
                    result['ur1_alias_checks'].append({'version': v, 'ip_j_rules': plain, 'ip_N_j_rules': numeric,
                        'rust_exit': checked.returncode, 'stdout': checked.stdout, 'stderr': checked.stderr})
                    if checked.returncode:
                        raise RuntimeError(f'v{v} aliased reserve table was not rejected')
                finally:
                    ip(v, 'rule', 'del', 'priority', '19001', 'lookup', '18402')
            finally:
                ip(v, 'route', 'del', 'default', 'dev', sinks[0 if v == 4 else 1].name, 'table', '18402')
        process = subprocess.run(['strace', '-f', '-e', 'trace=socket,setsockopt,getsockopt,connect,bind,listen,accept4,sendto,shutdown,close', '-o', '/run/syscalls.log', '/underlay-tests', '--exact', 'underlay::fixture::production_paths', '--ignored', '--nocapture'], env=env, capture_output=True, text=True, timeout=90)
        result['rust_stdout'], result['rust_stderr'], result['rust_exit'] = process.stdout, process.stderr, process.returncode
        if process.returncode:
            raise RuntimeError('production Rust fixture failed')
        def drop():
            os.setgroups([])
            os.setgid(65534)
            os.setuid(65534)
        denied = subprocess.run(['/underlay-tests', '--exact', 'underlay::fixture::permission_denied', '--ignored', '--nocapture'], env=env, preexec_fn=drop, capture_output=True, text=True, timeout=10)
        result['permission_stdout'], result['permission_stderr'], result['permission_exit'] = denied.stdout, denied.stderr, denied.returncode
        if denied.returncode:
            raise RuntimeError('permission denial did not fail closed')
        result['packets'] = [p for sink in sinks for p in sink.packets]
        if {p['version'] for p in result['packets'] if p['destination_port'] == 41861} != {4, 6}:
            raise RuntimeError('ordinary same-destination SYN missing from capture')
        # Protected sockets may not have leaked SYN, DNS, UDP or data to TUN.
        if len({(p['version'], p['source_port']) for p in result['packets']}) != 10:
            raise RuntimeError('unexpected protected socket or unmarked fallback captured')
        result['status'] = 'ok'
    except BaseException as error:
        result['error'] = str(error)
    finally:
        for v, kind in reversed(owned):
            args = {'capture': ['priority', '18001', 'iif', 'lo', 'lookup', '18401'],
                    'guard': ['priority', '18000', 'fwmark', '0x4d570000/0xffff0000', 'iif', 'lo', 'goto', '18003'],
                    'nop': ['priority', '18003', 'nop'],
                    'vpn': ['priority', '19000', 'fwmark', '0x20/0xff', 'lookup', '18550']}[kind]
            try:
                ip(v, 'rule', 'del', *args)
            except Exception as error:
                result['cleanup_errors'].append(str(error))
        for v in (4, 6):
            ip(v, 'route', 'del', 'default', 'dev', sinks[0 if v == 4 else 1].name, 'table', '18401', check=False)
            prefix = '172.30.247.0/24' if v == 4 else 'fd74:6d77:247::/64'
            ip(v, 'route', 'del', prefix, 'dev', 'eth0', 'table', '18550', check=False)
            after = json.loads(ip(v, '-j', 'rule', 'show').stdout)
            if before[v] != after:
                result['cleanup_errors'].append(f'v{v} RPDB differs after exact cleanup')
        for sink in sinks:
            sink.close()
        if result['cleanup_errors']:
            result['status'] = 'failed'
    print(json.dumps(result), flush=True)
    return 0 if result['status'] == 'ok' else 1


if __name__ == '__main__':
    server() if sys.argv[1] == 'server' else sys.exit(client())
