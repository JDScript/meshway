#!/usr/bin/env python3
"""Opt-in container-only DNS adapter; Python 3.11+ standard library."""

import argparse
import asyncio
import ipaddress
import json
import os
import signal
import socket
import struct
import sys
import time

from dns import DNSInvalid, Proxy, Router, forward, parse, question, zone_name
from state import Conflict, ResolverFile, Store, decoded, endpoint, process_start, read_fd, secure_open


def container_guard(args):
    if not args.allow_container_resolver:
        raise Conflict("explicit --allow-container-resolver is required")
    if sys.platform != "linux" or not os.path.isfile("/.dockerenv"):
        raise Conflict("run/down are supported only in Linux containers with /.dockerenv")


def run_settings(args):
    zone = zone_name(args.zone)
    listen, nameservice = endpoint(args.listen), endpoint(args.nameservice)
    if listen != ("127.0.0.1", 53):
        raise Conflict("CLI listener must be 127.0.0.1:53; public/alternate listeners are forbidden")
    if not ipaddress.ip_address(nameservice[0]).is_loopback or nameservice == listen:
        raise Conflict("nameservice must be a distinct numeric loopback endpoint")
    prefixes = [ipaddress.ip_network(args.mesh_ipv4_prefix, strict=True),
                ipaddress.ip_network(args.mesh_ipv6_prefix, strict=True)]
    if prefixes[0].version != 4 or prefixes[1].version != 6:
        raise Conflict("mesh prefix families must be IPv4 and IPv6 respectively")
    return zone, listen, nameservice, prefixes


async def run(args):
    container_guard(args)
    zone, listen, nameservice, prefixes = run_settings(args)
    with Store(args.state_dir) as store:
        store.lock()
        with ResolverFile(args.resolv_conf) as target:
            journal, previous = target.prepare(store, zone, listen, nameservice, prefixes)
            router = Router(zone, journal["zones"], prefixes, nameservice,
                            [tuple(p) for p in journal["upstreams"]])
            proxy = Proxy(router)
            stop = asyncio.Event()
            loop = asyncio.get_running_loop()
            for signum in (signal.SIGTERM, signal.SIGINT):
                loop.add_signal_handler(signum, stop.set)
            applied = False
            try:
                await proxy.bind(listen)  # Both UDP and TCP must bind first.
                async with asyncio.timeout(2):
                    # REFUSED/SERVFAIL/NXDOMAIN all prove that the local service
                    # is reachable, including a deliberately disabled service.
                    await forward(nameservice, question(zone))
                if stop.is_set():
                    return
                target.apply(store, journal, previous)
                applied = True
                print(json.dumps({"state": "active", "pid": os.getpid(), "zone": zone,
                                  "listen": list(listen)}), flush=True)
                await stop.wait()
            finally:
                # Stop serving before restoration, and retain retired zones on
                # graceful stop/reapply; explicit down clears that history.
                await proxy.close()
                for signum in (signal.SIGTERM, signal.SIGINT):
                    loop.remove_signal_handler(signum)
                if applied:
                    target.restore(store, journal)
                    print(json.dumps({"state": "restored"}), flush=True)


def probe(endpoint_value, zone, tcp=False):
    family = socket.AF_INET6 if ":" in endpoint_value[0] else socket.AF_INET
    data = question(zone)
    with socket.socket(family, socket.SOCK_STREAM if tcp else socket.SOCK_DGRAM) as sock:
        deadline = time.monotonic() + 2.3
        sock.settimeout(2.3)
        sock.connect(tuple(endpoint_value))
        if tcp:
            sock.sendall(struct.pack("!H", len(data)) + data)
            header = receive(sock, 2, deadline)
            length = struct.unpack("!H", header)[0]
            if not 12 <= length <= 4096:
                return False
            response = receive(sock, length, deadline)
        else:
            sock.send(data)
            response = sock.recv(4097)
        request, parsed = parse(data), parse(response)
        return (parsed.ident == request.ident and parsed.question == request.question
                and bool(parsed.flags & 0x8000) and not parsed.flags & 0x7800)


def receive(sock, count, deadline):
    result = b""
    while len(result) < count:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("status probe deadline")
        sock.settimeout(remaining)
        part = sock.recv(count - len(result))
        if not part:
            raise OSError("short TCP response")
        result += part
    return result


def status(args):
    result = dict(state="absent", journal=False, owner_lock=False, pid_alive=False,
                  listener_udp=False, listener_tcp=False, resolver_owned=False)
    try:
        store = Store(args.state_dir, create=False)
    except FileNotFoundError:
        print(json.dumps(result, sort_keys=True))
        return
    with store:
        journal = store.load()
        result["owner_lock"] = store.lock_busy()
        if result["owner_lock"] and journal is None:
            result["state"] = "preparing"
        if journal:
            result.update(journal=True, journal_phase=journal["phase"], zone=journal["zone"],
                          owned_zones=journal["zones"], pid=journal["pid"])
            result["pid_alive"] = bool(journal["pid"] and journal["process_start"]
                                       and process_start(journal["pid"]) == journal["process_start"])
            try:
                fd = secure_open(journal["path"], os.O_RDONLY)
                try:
                    info, data = os.fstat(fd), read_fd(fd)
                    same_inode = (info.st_dev, info.st_ino) == (journal["device"], journal["inode"])
                    result["resolver_owned"] = same_inode and data == decoded(journal["written"])
                    restored = same_inode and data == decoded(journal["original"])
                finally:
                    os.close(fd)
            except OSError:
                restored = False
            # Actual socket probes, independent of recorded phase. A listener
            # can exist even when the journal/PID is stale; do not call it active.
            for tcp, key in ((False, "listener_udp"), (True, "listener_tcp")):
                try:
                    result[key] = probe(journal["listen"], journal["zone"], tcp)
                except (OSError, DNSInvalid, ValueError):
                    pass
            result["state"] = "recovery-required" if result["resolver_owned"] else "conflict"
            if restored:
                result["state"] = "restored"
            if all(result[k] for k in ("owner_lock", "pid_alive", "listener_udp", "listener_tcp", "resolver_owned")):
                result["state"] = "active"
    print(json.dumps(result, sort_keys=True))


def down(args):
    container_guard(args)
    try:
        store = Store(args.state_dir, create=False)
    except FileNotFoundError:
        print(json.dumps({"state": "absent"}))
        return
    with store:
        store.lock()
        journal = store.load()
        if journal:
            with ResolverFile(args.resolv_conf) as target:
                target.restore(store, journal, forget=True)
        print(json.dumps({"state": "restored" if journal else "absent"}))


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    start = commands.add_parser("run", help="bind, verify nameservice, transactionally install container DNS")
    start.add_argument("--zone", required=True)
    start.add_argument("--mesh-ipv4-prefix", required=True)
    start.add_argument("--mesh-ipv6-prefix", required=True)
    start.add_argument("--listen", default="127.0.0.1:53")
    start.add_argument("--nameservice", default="127.0.0.1:5353")
    for cmd in (start, commands.add_parser("down", help="recover owner file and forget zone history")):
        cmd.add_argument("--state-dir", required=True)
        cmd.add_argument("--resolv-conf", default="/etc/resolv.conf")
        cmd.add_argument("--allow-container-resolver", action="store_true")
    inspect = commands.add_parser("status", help="read journal, process identity and actual listener probes")
    inspect.add_argument("--state-dir", required=True)
    return result


def main():
    args = parser().parse_args()
    try:
        if args.command == "run":
            asyncio.run(run(args))
        elif args.command == "down":
            down(args)
        else:
            status(args)
    except (Conflict, OSError, ValueError, TimeoutError, DNSInvalid) as exc:
        print(json.dumps({"state": "error", "error": str(exc) or type(exc).__name__}), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
