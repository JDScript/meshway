#!/usr/bin/env python3
"""Namespace-local acceptance helpers; no identities or config mutation."""
import http.server
import json
import os
from pathlib import Path
import socket
import socketserver
import subprocess
import sys
import threading
import time

ROOT = Path('/run/meshway-exit')


def save(name, value):
    path = ROOT / name
    tmp = path.with_suffix('.tmp')
    tmp.write_text(json.dumps(value))
    tmp.replace(path)


class HTTP(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({'source': self.client_address[0]}).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


class Echo(socketserver.BaseRequestHandler):
    def handle(self):
        while data := self.request.recv(65536):
            self.request.sendall(data)


class EchoServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def server():
    for service in [http.server.ThreadingHTTPServer(('0.0.0.0', 8080), HTTP),
                    EchoServer(('0.0.0.0', 9000), Echo)]:
        threading.Thread(target=service.serve_forever, daemon=True).start()
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp.bind(('0.0.0.0', 9001))
    count = 0
    save('udp.json', {'received': count})
    while True:
        data, address = udp.recvfrom(65536)
        count += 1
        save('udp.json', {'received': count, 'last_source': address[0], 'length': len(data)})
        udp.sendto(data, address)


def agent():
    child = subprocess.Popen(['meshway-exit-fixture', '--exact',
                             'managed_dataplane::exit::fixture::serve', '--ignored', '--nocapture'],
                            stdout=(ROOT / 'agent.log').open('w'), stderr=subprocess.STDOUT)
    # Namespace-local child PID, not a host PID; supervisor owns this child.
    ticks = Path(f'/proc/{child.pid}/stat').read_text().rsplit(')', 1)[1].split()[19]
    save('agent-pid.json', {'pid': child.pid, 'start_ticks': ticks})
    save('agent-exit.json', {'exit_code': child.wait()})


def flow(target, name):
    result = {'successes': 0, 'status': 'connecting'}
    save(name, result)
    try:
        with socket.create_connection((target, 9000), timeout=2) as stream:
            stream.settimeout(1)
            result['source_port'] = stream.getsockname()[1]
            while True:
                stream.sendall(b'meshway-exit-echo')
                received = b''
                while len(received) < len(b'meshway-exit-echo'):
                    data = stream.recv(len(b'meshway-exit-echo') - len(received))
                    if not data:
                        raise RuntimeError('echo closed')
                    received += data
                if received != b'meshway-exit-echo':
                    raise RuntimeError('echo closed or corrupted')
                result.update(successes=result['successes'] + 1, status='active', last_success_ns=time.monotonic_ns())
                save(name, result)
                time.sleep(.1)
    except (OSError, RuntimeError) as error:
        result.update(status='terminated', error=str(error))
        save(name, result)


if __name__ == '__main__':
    os.umask(0o077)
    if sys.argv[1] == 'server':
        server()
    elif sys.argv[1] == 'agent':
        agent()
    elif sys.argv[1] == 'flow':
        flow(sys.argv[2], sys.argv[3])
