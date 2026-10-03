"""Bounded transparent UDP impairment fixture; never a Meshway identity/peer."""
from pathlib import Path
import selectors
import socket
import sys
import time

selector = selectors.DefaultSelector()
listener = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
listener.bind(('0.0.0.0', 43101))
listener.setblocking(False)
selector.register(listener, selectors.EVENT_READ, None)
target = (socket.gethostbyname(sys.argv[1]), int(sys.argv[2]))
clients = {}
while True:
    for key, _ in selector.select(.2):
        if key.fileobj is listener:
            data, source = listener.recvfrom(65536)
            if Path('/run/drop-udp').exists():
                continue
            if source not in clients:
                if len(clients) >= 32:
                    continue
                outgoing = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                outgoing.connect(target)
                outgoing.setblocking(False)
                clients[source] = (outgoing, time.monotonic())
                selector.register(outgoing, selectors.EVENT_READ, source)
            outgoing, _ = clients[source]
            clients[source] = (outgoing, time.monotonic())
            outgoing.send(data)
        else:
            try:
                data = key.fileobj.recv(65536)
                if not Path('/run/drop-udp').exists():
                    listener.sendto(data, key.data)
            except ConnectionRefusedError:
                pass
    for source, (outgoing, touched) in list(clients.items()):
        if time.monotonic() - touched > 60:
            selector.unregister(outgoing)
            outgoing.close()
            del clients[source]
