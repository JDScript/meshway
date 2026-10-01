"""Runs only inside disposable Docker network namespaces."""
import fcntl
import json
import os
from pathlib import Path
import select
import signal
import socket
import ssl
import struct
import subprocess
import sys
import threading
import time
import traceback

from helper import Reservation, RouteGuard, fresh, parse_packet, parse_stun, stun_reply, stun_request

PORTS = {'http': 41861, 'tls': 41862, 'stun': 41863, 'dns': 41864, 'probe': 41865}
SERVER = {4: '172.30.247.20', 6: 'fd74:6d77:247::20'}
CLIENT = {4: '172.30.247.10', 6: 'fd74:6d77:247::10'}
NEW = {4: '172.30.247.11', 6: 'fd74:6d77:247::11'}
EXIT = {4: '172.30.247.13', 6: 'fd74:6d77:247::13'}
EXIT_F = {4: '172.30.247.14', 6: 'fd74:6d77:247::14'}


def ip(version, *args, check=True):
    p = subprocess.run(['ip', '-' + str(version), *args], capture_output=True, text=True, timeout=5)
    if check and p.returncode:
        raise RuntimeError(f'ip {args}: {p.stderr}')
    return p


def dns_reply(request):
    # Standard one-question DNS fixture; no product resolver/DTO.
    if len(request) < 17 or request[4:6] != b'\x00\x01':
        raise ValueError('invalid DNS question')
    end = 12
    while request[end]:
        end += request[end] + 1
    end += 1
    qtype, qclass = struct.unpack('!HH', request[end:end + 4])
    if qclass != 1 or qtype not in (1, 28):
        raise ValueError('unsupported fixture DNS question')
    address = socket.inet_pton(socket.AF_INET if qtype == 1 else socket.AF_INET6, SERVER[4 if qtype == 1 else 6])
    return (request[:2] + struct.pack('!HHHHH', 0x8180, 1, 1, 0, 0) + request[12:end + 4]
            + b'\xc0\x0c' + struct.pack('!HHIH', qtype, 1, 30, len(address)) + address)


def server():
    def record(event):
        with open('/run/observer.jsonl', 'a') as output:
            output.write(json.dumps(event) + '\n')

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain('/fixture.crt', '/fixture.key')

    def udp(version, service):
        sock = socket.socket(socket.AF_INET if version == 4 else socket.AF_INET6, socket.SOCK_DGRAM)
        sock.bind((SERVER[version], PORTS[service]))
        while True:
            request, peer = sock.recvfrom(4096)
            if service == 'stun':
                answer = stun_reply(request, peer)
            elif service == 'dns':
                answer = dns_reply(request)
            else:
                answer = request  # nonce round trip, NOT production authenticated probe.
            record({'service': service, 'recvfrom': list(peer), 'version': version,
                    **({'transaction': request[8:20].hex()} if service == 'stun' else {})})
            sock.sendto(answer, peer)

    def tcp(version, service):
        sock = socket.socket(socket.AF_INET if version == 4 else socket.AF_INET6, socket.SOCK_STREAM)
        sock.bind((SERVER[version], PORTS[service]))
        sock.listen()
        while True:
            conn, peer = sock.accept()
            try:
                conn.settimeout(2)
                if service == 'tls':
                    conn = context.wrap_socket(conn, server_side=True)
                request = conn.recv(4096)
                if service == 'dns':
                    length = struct.unpack('!H', request[:2])[0]
                    answer = dns_reply(request[2:2 + length])
                    conn.sendall(struct.pack('!H', len(answer)) + answer)
                else:
                    payload = json.dumps({'source': peer[0], 'service': service}).encode()
                    conn.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: ' + str(len(payload)).encode()
                                 + b'\r\nConnection: close\r\n\r\n' + payload)
                record({'service': service, 'accept': list(peer), 'version': version})
            except (OSError, ValueError):
                pass
            finally:
                conn.close()

    for version in (4, 6):
        for service in ('stun', 'dns', 'probe'):
            threading.Thread(target=udp, args=(version, service), daemon=True).start()
        for service in ('http', 'tls', 'dns'):
            threading.Thread(target=tcp, args=(version, service), daemon=True).start()
    print('ready', flush=True)
    signal.pause()


class TunSink:
    def __init__(self, name):
        self.name, self.packets, self.errors = name, [], []
        self.fd = os.open('/dev/net/tun', os.O_RDWR | os.O_NONBLOCK)
        fcntl.ioctl(self.fd, 0x400454CA, struct.pack('16sH', name.encode(), 0x0001 | 0x1000))
        ip(4, 'link', 'set', name, 'up')
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.read, daemon=True)
        self.thread.start()

    def read(self):
        while not self.stopped.is_set():
            if select.select([self.fd], [], [], .05)[0]:
                try:
                    packet = os.read(self.fd, 65535)
                    self.packets.append(parse_packet(packet))
                except ValueError:
                    pass  # ND/control packets aren't TCP/UDP evidence.
                except OSError as error:
                    self.errors.append(str(error))

    def close(self):
        self.stopped.set()
        self.thread.join(1)
        os.close(self.fd)


def exchange(guard, version, service, *, source=None, existing_mark=0, protected=True):
    family = socket.AF_INET if version == 4 else socket.AF_INET6
    kind = socket.SOCK_DGRAM if service in ('stun', 'probe', 'dns') else socket.SOCK_STREAM
    purpose = {'http': 'controller', 'tls': 'carrier', 'stun': 'stun',
               'probe': 'authenticated_probe', 'dns': 'bootstrap_dns', 'dns_tcp': 'bootstrap_dns'}[service]
    if protected:
        sock = guard.socket(purpose, family, kind, source=source, existing_mark=existing_mark)
    else:
        sock = socket.socket(family, kind)
        sock.settimeout(.35)
        if source:
            sock.bind((source, 0))
    base = 'dns' if service == 'dns_tcp' else service
    endpoint = (SERVER[version], PORTS[base])
    transaction = os.urandom(12)
    request = stun_request(transaction) if service == 'stun' else b'fixture-nonce-' + transaction
    if base == 'dns':
        request = (b'\x73\x19' + struct.pack('!HHHHH', 0x100, 1, 0, 0, 0)
                   + b'\x08underlay\x07fixture\x00' + struct.pack('!HH', 1 if version == 4 else 28, 1))
    if service in ('http', 'tls'):
        request = b'GET / HTTP/1.1\r\nHost: underlay.fixture\r\nConnection: close\r\n\r\n'
    result = {'service': service, 'version': version, 'protected': protected}
    result['mark_before_io'] = sock.getsockopt(socket.SOL_SOCKET, socket.SO_MARK)
    try:
        if kind == socket.SOCK_STREAM:
            sock.connect(endpoint)  # SO_MARK already set; includes SYN and TLS handshake.
            if service == 'tls':
                context = ssl.create_default_context(cafile='/fixture.crt')
                sock = context.wrap_socket(sock, server_hostname='underlay.fixture')
            sock.sendall((struct.pack('!H', len(request)) if service == 'dns_tcp' else b'') + request)
            reply = sock.recv(4096)
            if service in ('http', 'tls'):
                while b'\r\n\r\n' not in reply:
                    reply += sock.recv(4096)
                head, payload = reply.split(b'\r\n\r\n', 1)
                length = int(head.split(b'Content-Length: ')[1].split(b'\r\n')[0])
                while len(payload) < length:
                    payload += sock.recv(4096)
                result['server'] = json.loads(payload)
            else:
                reply = reply[2:]
        else:
            sock.sendto(request, endpoint)  # SO_MARK already set before first send.
            reply, responder = sock.recvfrom(4096)
            if responder[0:2] != endpoint:
                raise ValueError('unexpected observer')
        result['local'] = list(sock.getsockname())
        if service == 'stun':
            result['mapped'] = list(parse_stun(reply, transaction))
        elif base == 'dns':
            result['dns_answer'] = socket.inet_ntop(family, reply[-(4 if version == 4 else 16):])
            if reply[:2] != request[:2] or result['dns_answer'] != SERVER[version]:
                raise ValueError('invalid DNS answer')
        elif service == 'probe' and reply != request:
            raise ValueError('nonce mismatch')
        result['status'] = 'ok'
    except (OSError, ValueError) as error:
        result.update(status='failed', error=str(error))
    finally:
        try:
            result['local'] = list(sock.getsockname())
        except OSError:
            pass
        sock.close()
    return result


def client():
    checks, cleanup_errors, commands = [], [], []
    result = {'status': 'failed', 'checks': checks, 'cleanup_errors': cleanup_errors, 'commands': commands}
    reservation = Reservation()
    guards, sinks, owned_rules, owned_routes = [], [], [], []

    def check(name, success, **evidence):
        checks.append({'name': name, 'status': 'ok' if success else 'failed', **evidence})
        if not success:
            raise RuntimeError(name)

    def mutate(version, *args):
        commands.append(['ip', '-' + str(version), *args])
        return ip(version, *args)

    def add_rule(version, *args):
        mutate(version, 'rule', 'add', *args)
        owned_rules.append((version, args))

    def add_route(version, *args):
        mutate(version, 'route', 'add', *args)
        owned_routes.append((version, args))

    try:
        for version in (4, 6):
            # Fixture-only second underlay address and simulated E source. No host changes.
            for address in (NEW[version], EXIT[version], EXIT_F[version]):
                mutate(version, 'addr', 'add', address + ('/24' if version == 4 else '/64'),
                       'dev', 'eth0', *(['nodad'] if version == 6 else []))
        deadline = time.monotonic() + 5
        while True:
            addresses = json.loads(ip(6, '-j', 'addr', 'show', 'dev', 'eth0').stdout)[0]['addr_info']
            if any(a.get('dadfailed') or 'dadfailed' in a.get('flags', []) for a in addresses):
                raise RuntimeError('fixture IPv6 DAD failed')
            if not any(a.get('tentative') or 'tentative' in a.get('flags', []) for a in addresses):
                break
            if time.monotonic() >= deadline:
                raise RuntimeError('fixture IPv6 DAD not ready')
            time.sleep(.05)
        snapshots = {v: {'rules': json.loads(ip(v, '-j', 'rule', 'show').stdout),
                         'routes': json.loads(ip(v, '-j', 'route', 'show', 'table', 'all').stdout)} for v in (4, 6)}
        result['original_routing_snapshots'] = snapshots
        for version in (4, 6):
            reservation.validate(snapshots[version]['rules'], set())
            # Live collision refusal: leave a foreign-owned table/rule intact, refuse
            # admission, then remove only the exact fixture-created conflict.
            add_route(version, 'unreachable', 'default', 'table', '18401')
            before = ip(version, '-j', 'route', 'show', 'table', '18401').stdout
            try:
                reservation.validate(snapshots[version]['rules'], {18401})
                refused = False
            except ValueError:
                refused = True
            check(f'v{version} occupied table refused intact', refused
                  and before == ip(version, '-j', 'route', 'show', 'table', '18401').stdout,
                  foreign_table=json.loads(before))
            mutate(version, 'route', 'del', 'unreachable', 'default', 'table', '18401')
            owned_routes.pop()
            add_rule(version, 'pref', '18000', 'lookup', '18551')
            rules = json.loads(ip(version, '-j', 'rule', 'show').stdout)
            try:
                reservation.validate(rules, set())
                refused = False
            except ValueError:
                refused = True
            check(f'v{version} occupied priority refused intact', refused
                  and rules == json.loads(ip(version, '-j', 'rule', 'show').stdout), foreign_rules=rules)
            mutate(version, 'rule', 'del', 'pref', '18000', 'lookup', '18551')
            owned_rules.pop()
            add_rule(version, 'pref', '19020', 'fwmark', '0x10000/0xffff0000', 'lookup', '18551')
            rules = json.loads(ip(version, '-j', 'rule', 'show').stdout)
            try:
                reservation.validate(rules, set())
                refused = False
            except ValueError:
                refused = True
            check(f'v{version} overlapping mark owner refused intact', refused
                  and rules == json.loads(ip(version, '-j', 'rule', 'show').stdout), foreign_rules=rules)
            mutate(version, 'rule', 'del', 'pref', '19020', 'fwmark', '0x10000/0xffff0000', 'lookup', '18551')
            owned_rules.pop()
        guard = RouteGuard(reservation, 1)
        guard.prepare(snapshots[4]['rules'], set())
        guard.activate()
        guards.append(guard)
        for version in (4, 6):
            baseline = exchange(guard, version, 'stun', source=CLIENT[version])
            check(f'v{version} pre-capture real STUN', baseline['status'] == 'ok'
                  and baseline['mapped'][0] == CLIENT[version], response=baseline)
            contrast = exchange(guard, version, 'stun', source=EXIT[version])
            check(f'v{version} simulated E has distinct observed address', contrast['mapped'][0] == EXIT[version], response=contrast)
            contrast_f = exchange(guard, version, 'stun', source=EXIT_F[version])
            check(f'v{version} simulated F has distinct observed address', contrast_f['mapped'][0] == EXIT_F[version], response=contrast_f)

        sinks = [TunSink('mwua'), TunSink('mwub')]
        for version in (4, 6):
            # Initially specific prefix; then switch to /0. No endpoint bypass route.
            prefix = '172.30.247.0/24' if version == 4 else 'fd74:6d77:247::/64'
            for table, sink in zip(reservation.tables, sinks):
                add_route(version, prefix, 'dev', sink.name, 'table', str(table), 'metric', '10')
                add_route(version, 'unreachable', prefix, 'table', str(table), 'metric', '32000')
            source = '172.30.247.21' if version == 4 else 'fd74:6d77:247::21'
            add_rule(version, 'pref', '18001', 'not', 'fwmark', '0x4d570000/0xffff0000', 'iif', 'lo', 'lookup', '18401')
            counterexample = ip(version, 'route', 'get', SERVER[version], 'from', source, 'iif', 'eth0')
            check(f'v{version} negated combined selector counterexample captures forwarding',
                  'mwua' in counterexample.stdout and '18401' in counterexample.stdout,
                  route_get=counterexample.stdout)
            mutate(version, 'rule', 'del', 'pref', '18001', 'not', 'fwmark', '0x4d570000/0xffff0000',
                   'iif', 'lo', 'lookup', '18401')
            owned_rules.pop()
            # RPDB "not" inverts the ENTIRE selector, so not fwmark+iif lo would
            # accidentally match forwarded traffic. Positive goto skips ONLY our
            # contiguous capture block, then resumes all original system policy.
            # Linux requires an exact-priority goto target; unresolved goto would
            # fall through. Install our stable NOP target before the guard/capture.
            add_rule(version, 'pref', '18003', 'nop')
            add_rule(version, 'pref', '18000', 'fwmark', '0x4d570000/0xffff0000', 'iif', 'lo', 'goto', '18003')
            add_rule(version, 'pref', '18001', 'iif', 'lo', 'lookup', '18401')
            actual_rules = json.loads(ip(version, '-j', 'rule', 'show').stdout)
            jump = next(r for r in actual_rules if r['priority'] == 18000)
            check(f'v{version} guard goto is resolved to stable NOP target', jump.get('goto') == 18003
                  and 'unresolved' not in jump, rules=actual_rules)
            forwarded = ip(version, 'route', 'get', SERVER[version], 'from', source, 'iif', 'eth0')
            check(f'v{version} forwarded lookup is not captured by local business policy',
                  'eth0' in forwarded.stdout and 'mwua' not in forwarded.stdout and '18401' not in forwarded.stdout,
                  route_get=forwarded.stdout)

        def matrix(label, sink):
            for version in (4, 6):
                for service in ('http', 'stun', 'tls', 'dns', 'dns_tcp', 'probe'):
                    start = len(sink.packets)
                    protected = exchange(guard, version, service, source=CLIENT[version])
                    check(f'{label} v{version} protected {service}', protected['status'] == 'ok', response=protected)
                    leaked = [p for p in sink.packets[start:] if p['version'] == version
                              and p['destination'] == SERVER[version]
                              and p['source_port'] == protected['local'][1]]
                    check(f'{label} v{version} protected {service} absent from TUN', not leaked, tun_packets=leaked)
                    normal = exchange(guard, version, service, source=CLIENT[version], protected=False)
                    time.sleep(.05)
                    matching = [p for p in sink.packets[start:] if p['version'] == version
                                and p['destination'] == SERVER[version]
                                and p['destination_port'] == PORTS['dns' if service == 'dns_tcp' else service]]
                    check(f'{label} v{version} same-process same-host same-port {service} business captured',
                          normal['status'] == 'failed' and bool(matching), response=normal, tun_packets=matching)
                    if service == 'stun':
                        check(f'{label} v{version} STUN not selected E/F', protected['mapped'][0] == CLIENT[version]
                              and protected['mapped'][0] not in (EXIT[version], EXIT_F[version]),
                              response=protected, simulated_selected_exit='F' if label == 'profile-B' else 'E')

        matrix('specific', sinks[0])
        for version in (4, 6):
            for table in reservation.tables:
                prefix = 'default'
                add_route(version, prefix, 'dev', 'mwua' if table == 18401 else 'mwub', 'table', str(table), 'metric', '10')
                add_route(version, 'unreachable', prefix, 'table', str(table), 'metric', '32000')
        matrix('default', sinks[0])

        # Existing system policy uses separate low mark bits and routes through its own
        # table, deliberately choosing a different source from main. Marked Meshway
        # sockets must preserve those bits and reach that policy, not force main.
        for version in (4, 6):
            prefix = '172.30.247.0/24' if version == 4 else 'fd74:6d77:247::/64'
            add_route(version, prefix, 'dev', 'eth0', 'src', NEW[version], 'table', '18550')
            add_rule(version, 'pref', '19000', 'fwmark', '0x20/0xff', 'lookup', '18550')
            before = ip(version, '-j', 'rule', 'show').stdout
            protected = exchange(guard, version, 'stun', existing_mark=0x20)
            route = ip(version, 'route', 'get', SERVER[version], 'mark', '0x4d570020').stdout
            check(f'v{version} original system policy and low mark preserved', protected['status'] == 'ok'
                  and protected['mapped'][0] == NEW[version] and '18550' in route,
                  response=protected, route_get=route, foreign_rule=before)

        foreign_policy = {v: {'rules': [r for r in json.loads(ip(v, '-j', 'rule', 'show').stdout)
                                       if r['priority'] == 19000],
                              'routes': json.loads(ip(v, '-j', 'route', 'show', 'table', '18550').stdout)}
                          for v in (4, 6)}

        # Underlay itself denied: marked STUN must fail, never retry unmarked into TUN.
        for version in (4, 6):
            add_rule(version, 'pref', '19010', 'fwmark', '0x4d570000/0xffff0000', 'unreachable')
            start = len(sinks[0].packets)
            denied = exchange(guard, version, 'stun', source=CLIENT[version])
            time.sleep(.05)
            leaked = [p for p in sinks[0].packets[start:] if p['version'] == version
                      and p['source_port'] == denied['local'][1] and p['destination_port'] == PORTS['stun']]
            check(f'v{version} missing underlay path produces no candidate or TUN fallback',
                  denied['status'] == 'failed' and 'mapped' not in denied and not leaked,
                  response=denied, candidate=None, tun_packets=leaked)
            mutate(version, 'rule', 'del', 'pref', '19010', 'fwmark', '0x4d570000/0xffff0000', 'unreachable')
            owned_rules.pop()

        # New independent profile table is ready before old capture lookup is removed.
        for version in (4, 6):
            add_rule(version, 'pref', '18002', 'iif', 'lo', 'lookup', '18402')
            old = next(entry for entry in owned_rules if entry[0] == version and entry[1][1] == '18001')
            mutate(version, 'rule', 'del', *old[1])
            owned_rules.remove(old)
        matrix('profile-B', sinks[1])

        old_observation = {'generation': 1, 'observed_at': time.monotonic(), 'expires_at': time.monotonic() + 30}
        new_guard = RouteGuard(reservation, 2)
        new_guard.prepare(snapshots[4]['rules'], set())
        guards.append(new_guard)
        sequence = ['prepare-new-protected-path']
        old_socket = guard.socket('stun', socket.AF_INET, socket.SOCK_DGRAM, source=CLIENT[4])
        old_socket.sendto(stun_request(b'old-sent-123'), (SERVER[4], PORTS['stun']))
        result['pre_retirement_send'] = {'transaction': b'old-sent-123'.hex(), 'local': list(old_socket.getsockname())}
        # Already-sent bytes may arrive; this is not a production packet-generation fence.
        sequence.append('invalidate-old-observation-and-close-old-sockets')
        guard.retire()
        try:
            old_socket.sendto(stun_request(b'unsent-12345'), (SERVER[4], PORTS['stun']))
            refused = False
        except OSError as error:
            refused = True
            retired_error = str(error)
        check('retired socket cannot send unsent packet', refused, error=retired_error)
        check('retired observation not fresh', not fresh(old_observation, guard),
              observation=old_observation, active=guard.active, generation=guard.generation)
        new_guard.activate()
        sequence.append('activate-new-sockets')
        for version in (4, 6):
            answer = exchange(new_guard, version, 'stun', source=NEW[version])
            check(f'v{version} new network-generation source observed', answer['status'] == 'ok'
                  and answer['mapped'][0] == NEW[version], response=answer, sequence=sequence)
        check('old observation cannot become fresh in new generation', not fresh(old_observation, new_guard),
              observation=old_observation, active=new_guard.active, generation=new_guard.generation)

        inactive = RouteGuard(reservation, 3)
        try:
            inactive.socket('stun', socket.AF_INET, socket.SOCK_DGRAM)
            refused = False
        except RuntimeError:
            refused = True
        check('missing protection rejects candidate before socket creation', refused, candidate=None)
        child = subprocess.run([sys.executable, __file__, 'permission-denied'], capture_output=True, text=True, timeout=5)
        check('SO_MARK permission failure has no unmarked fallback/candidate', child.returncode == 0,
              evidence=child.stdout, candidate=None)

        # Close TUNs: kernel removes dev routes, unreachable stays; protection still works.
        for sink in sinks:
            sink.close()
        sinks = []
        for version in (4, 6):
            marked = exchange(new_guard, version, 'http', source=NEW[version])
            business = exchange(new_guard, version, 'http', source=NEW[version], protected=False)
            route = ip(version, 'route', 'get', SERVER[version], check=False)
            check(f'v{version} TUN loss business fail-closed while control stays reachable',
                  marked['status'] == 'ok' and business['status'] == 'failed' and route.returncode != 0,
                  protected=marked, ordinary=business, route_error=route.stderr)
            remaining = {'rules': [r for r in json.loads(ip(version, '-j', 'rule', 'show').stdout)
                                    if r['priority'] == 19000],
                         'routes': json.loads(ip(version, '-j', 'route', 'show', 'table', '18550').stdout)}
            check(f'v{version} original non-Meshway policy survives switches and TUN loss',
                  remaining == foreign_policy[version], before=foreign_policy[version], after=remaining)
        result['status'] = 'ok'
    except BaseException as error:
        result['error'] = str(error)
        result['traceback'] = traceback.format_exc()
    finally:
        for guard in guards:
            guard.retire()
        for sink in sinks:
            sink.close()
        # Exact owned records only. No rule/table flush; missing dev routes are expected.
        for version, args in reversed(owned_rules):
            response = ip(version, 'rule', 'del', *args, check=False)
            if response.returncode:
                cleanup_errors.append(response.stderr)
        for version, args in reversed(owned_routes):
            response = ip(version, 'route', 'del', *args, check=False)
            if response.returncode and not any(text in response.stderr for text in ('No such process', 'Cannot find device')):
                cleanup_errors.append(response.stderr)
        result['final_rules'] = {str(v): json.loads(ip(v, '-j', 'rule', 'show').stdout) for v in (4, 6)}
        result['final_routes'] = {str(v): json.loads(ip(v, '-j', 'route', 'show', 'table', 'all').stdout) for v in (4, 6)}
        if 'snapshots' in locals():
            for version in (4, 6):
                if result['final_rules'][str(version)] != snapshots[version]['rules']:
                    cleanup_errors.append(f'v{version} RPDB not restored')
                normalize = lambda rows: sorted(json.dumps(row, sort_keys=True) for row in rows)
                if normalize(result['final_routes'][str(version)]) != normalize(snapshots[version]['routes']):
                    cleanup_errors.append(f'v{version} original routes not restored')
        if cleanup_errors:
            result['status'] = 'failed'
        print(json.dumps(result), flush=True)
    return 0 if result['status'] == 'ok' else 1


def permission_denied():
    os.setgroups([])
    os.setgid(65534)
    os.setuid(65534)
    guard = RouteGuard(Reservation(), 1)
    guard.prepare([], set())
    guard.activate()
    try:
        guard.socket('stun', socket.AF_INET, socket.SOCK_DGRAM)
    except PermissionError as error:
        print(json.dumps({'status': 'refused', 'error': str(error), 'candidate': None}))
        return 0
    return 1


if __name__ == '__main__':
    mode = sys.argv[1]
    if mode == 'server':
        server()
    elif mode == 'permission-denied':
        sys.exit(permission_denied())
    else:
        sys.exit(client())
