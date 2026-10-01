"""Host tests: only private temporary files and ephemeral loopback sockets."""

import asyncio
import contextlib
import io
import ipaddress
import json
import os
from pathlib import Path
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import dns
import resolver
import state

HERE = Path(__file__).parent.resolve()
ORIGINAL = b"# original\nnameserver 127.0.0.11\nnameserver 2001:db8::53\nsearch old.test FIXTURE.meshway.test\noptions ndots:0 timeout:1 attempts:2\n"
PREFIXES = [ipaddress.ip_network("100.96.0.0/16"), ipaddress.ip_network("fd96::/64")]


def answer(data, ttl=60, address="100.96.0.2", rcode=0, tc=False):
    parsed = dns.parse(data)
    flags = 0x8180 | rcode | (0x200 if tc else 0)
    header = struct.pack("!6H", parsed.ident, flags, 1, 0 if rcode or tc else 1, 0, 0)
    result = header + data[12:parsed.question_end]
    if not rcode and not tc:
        result += b"\xc0\x0c" + struct.pack("!HHIH", 1, 1, ttl, 4) + ipaddress.IPv4Address(address).packed
    return result


class Fixture(asyncio.DatagramProtocol):
    def __init__(self, callback=None, delay=0, ttl=60, rcode=0):
        self.callback, self.delay, self.ttl, self.rcode = callback, delay, ttl, rcode
        self.requests, self.tcp_requests, self.tasks = [], [], set()
        self.transport = self.server = None

    async def start(self, tcp=True):
        self.transport, _ = await asyncio.get_running_loop().create_datagram_endpoint(
            lambda: self, local_addr=("127.0.0.1", 0))
        self.endpoint = self.transport.get_extra_info("sockname")[:2]
        if tcp:
            self.server = await asyncio.start_server(self.tcp, *self.endpoint)
        return self

    def datagram_received(self, data, addr):
        self.requests.append(data)
        task = asyncio.create_task(self.send(data, addr))
        self.tasks.add(task)
        task.add_done_callback(self.tasks.discard)

    async def send(self, data, addr):
        if self.delay:
            await asyncio.sleep(self.delay)
        if self.callback:
            await self.callback(self, data, addr)
        else:
            self.transport.sendto(answer(data, self.ttl, rcode=self.rcode), addr)

    async def tcp(self, reader, writer):
        try:
            size = struct.unpack("!H", await reader.readexactly(2))[0]
            data = await reader.readexactly(size)
            self.tcp_requests.append(data)
            response = answer(data, self.ttl, rcode=self.rcode)
            writer.write(struct.pack("!H", len(response)) + response)
            await writer.drain()
        finally:
            writer.close()

    async def close(self):
        if self.transport:
            self.transport.close()
        if self.server:
            self.server.close()
            await self.server.wait_closed()
        for task in list(self.tasks):
            task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)


class DNSBehavior(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.fixtures, self.proxies = [], []

    async def asyncTearDown(self):
        for proxy in self.proxies:
            await proxy.close()
        for fixture in self.fixtures:
            await fixture.close()

    async def fixture(self, **kwargs):
        fixture = await Fixture(**kwargs).start()
        self.fixtures.append(fixture)
        return fixture

    async def proxy(self, local, external, previous=()):
        router = dns.Router("fixture.meshway.test", previous, PREFIXES,
                            local.endpoint, [external.endpoint])
        proxy = dns.Proxy(router)
        proxy.endpoint = await proxy.bind(("127.0.0.1", 0))
        self.proxies.append(proxy)
        return proxy

    async def query(self, proxy, name, kind=1, tcp=False):
        data = dns.question(name, kind, 413)
        if tcp:
            async with asyncio.timeout(3):
                return await dns.tcp_exchange(proxy.endpoint, data, dns.parse(data))
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.setblocking(False)
            await loop.sock_sendto(sock, data, proxy.endpoint)
            async with asyncio.timeout(3):
                response, _ = await loop.sock_recvfrom(sock, 65535)
                return response

    async def test_real_udp_tcp_routing_boundary_and_unknown_mesh_ptr(self):
        local, external = await self.fixture(), await self.fixture()
        proxy = await self.proxy(local, external, ["retired.meshway.test"])
        for name in ("node.fixture.meshway.test", "fixture.meshway.test",
                     "2.0.96.100.in-addr.arpa", "250.255.96.100.in-addr.arpa",
                     ipaddress.IPv6Address("fd96::dead").reverse_pointer):
            response = await self.query(proxy, name, tcp=name.startswith("node"))
            self.assertEqual(dns.parse(response).ident, 413)
        self.assertEqual(len(local.requests), 5)
        self.assertFalse(external.requests)
        for name in ("notfixture.meshway.test", "fixture.meshway.test.external",
                     "2.0.97.100.in-addr.arpa", ipaddress.IPv6Address("fd97::1").reverse_pointer,
                     "controller.mesh.test", "999.0.96.100.in-addr.arpa"):
            await self.query(proxy, name)
        self.assertEqual(len(external.requests), 6)
        for name in ("solo", "docker-service"):
            response = await self.query(proxy, name)
            self.assertEqual(dns.parse(response).flags & 15, 5)
        self.assertEqual(len(external.requests), 6)
        response = await self.query(proxy, "node.retired.meshway.test")
        self.assertEqual(dns.parse(response).flags & 15, 5)
        self.assertEqual(len(external.requests), 6)

    async def test_local_error_codes_and_disabled_never_fallback(self):
        external = await self.fixture()
        for rcode in (2, 3, 5):
            local = await self.fixture(rcode=rcode)
            proxy = await self.proxy(local, external)
            for name in ("node.fixture.meshway.test", "9.0.96.100.in-addr.arpa"):
                result = await self.query(proxy, name)
                self.assertEqual(dns.parse(result).flags & 15, rcode)
        self.assertFalse(external.requests)

    async def test_original_search_cannot_turn_unknown_mesh_short_name_into_external_alias(self):
        local, external = await self.fixture(rcode=3), await self.fixture()
        proxy = await self.proxy(local, external)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path = root / "resolv.conf"
            original = ORIGINAL + b"domain legacy.test\nsearch old.test another.test\n"
            path.write_bytes(original)
            with state.Store(root / "private") as store, state.ResolverFile(path) as target:
                store.lock()
                journal, previous = target.prepare(store, "fixture.meshway.test",
                                                   ("127.0.0.1", 53), ("127.0.0.1", 5353), PREFIXES)
                target.apply(store, journal, previous)
                directives = [line.split() for line in path.read_text().splitlines()]
                searches = [parts[1:] for parts in directives if parts and parts[0] == "search"]
                self.assertEqual(searches, [["fixture.meshway.test"]])
                self.assertFalse(any(parts and parts[0] == "domain" for parts in directives))
                # Exercise the installed search candidates and final bare-name
                # retry over actual UDP. This is not a glibc implementation.
                for suffix in searches[0]:
                    result = await self.query(proxy, "solo." + suffix)
                    self.assertEqual(dns.parse(result).flags & 15, 3)
                result = await self.query(proxy, "solo")
                self.assertEqual(dns.parse(result).flags & 15, 5)
                self.assertFalse(external.requests)
                # An explicit external FQDN is still allowed. The Router cannot
                # infer that solo.old.test originated from a libc search retry.
                result = await self.query(proxy, "solo.old.test")
                self.assertEqual(dns.parse(result).flags & 15, 0)
                self.assertEqual(len(external.requests), 1)
                self.assertEqual(state.decoded(store.load()["original"]), original)
                target.restore(store, journal, forget=True)
                self.assertEqual(path.read_bytes(), original)

    async def test_wrong_id_question_opcode_peer_and_record_bounds_are_ignored(self):
        async def callback(fixture, data, addr):
            good = answer(data)
            bad_id = struct.pack("!H", (dns.parse(data).ident + 1) % 65536) + good[2:]
            bad_question = answer(dns.question("other.fixture.meshway.test", ident=dns.parse(data).ident))
            bad_opcode = good[:2] + struct.pack("!H", 0x8980) + good[4:]
            for invalid in (bad_id, bad_question, bad_opcode, good[:-1], good + b"x", b"x" * 4097):
                fixture.transport.sendto(invalid, addr)
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as stranger:
                stranger.sendto(answer(data, address="100.96.0.99"), addr)
            await asyncio.sleep(0.02)
            fixture.transport.sendto(good, addr)

        local, external = await self.fixture(callback=callback), await self.fixture()
        proxy = await self.proxy(local, external)
        response = await self.query(proxy, "node.fixture.meshway.test")
        self.assertEqual(response[-4:], ipaddress.IPv4Address("100.96.0.2").packed)
        self.assertEqual(dns.parse(response).ident, 413)
        self.assertFalse(external.requests)

    async def test_udp_tc_uses_tcp_same_transaction_and_returns_client_id(self):
        async def callback(fixture, data, addr):
            fixture.transport.sendto(answer(data, tc=True), addr)
        local, external = await self.fixture(callback=callback), await self.fixture()
        proxy = await self.proxy(local, external)
        response = await self.query(proxy, "node.fixture.meshway.test")
        self.assertEqual(dns.parse(response).ident, 413)
        self.assertEqual(local.requests[0], local.tcp_requests[0])
        self.assertEqual(dns.parse(response).flags & 0x200, 0)

    async def test_ttl_is_aged_and_expired_positive_is_not_reissued(self):
        local, external = await self.fixture(delay=0.04, ttl=5), await self.fixture()
        proxy = await self.proxy(local, external)
        response = await self.query(proxy, "node.fixture.meshway.test")
        self.assertEqual(dns.parse(response).records[0][1], 4)
        local.ttl, local.delay = 1, 1.08
        response = await self.query(proxy, "node.fixture.meshway.test")
        self.assertEqual(dns.parse(response).flags & 15, 2)
        self.assertFalse(dns.parse(response).records)
        self.assertFalse(external.requests)

    async def test_no_cache_and_no_transfer_or_any(self):
        local, external = await self.fixture(), await self.fixture()
        proxy = await self.proxy(local, external)
        for _ in range(2):
            await self.query(proxy, "node.fixture.meshway.test")
        self.assertEqual(len(local.requests), 2)
        for kind in (251, 252, 255):
            response = await self.query(proxy, "node.fixture.meshway.test", kind)
            self.assertEqual(dns.parse(response).flags & 15, 5)
        self.assertEqual(len(local.requests), 2)

    async def test_deadline_and_partial_tcp_closed(self):
        local, external = await self.fixture(delay=3), await self.fixture()
        proxy = await self.proxy(local, external)
        started = time.monotonic()
        response = await self.query(proxy, "node.fixture.meshway.test")
        self.assertEqual(dns.parse(response).flags & 15, 2)
        self.assertLess(time.monotonic() - started, 2.5)
        response = await self.query(proxy, "node.fixture.meshway.test", tcp=True)
        self.assertEqual(dns.parse(response).flags & 15, 2)
        reader, writer = await asyncio.open_connection(*proxy.endpoint)
        writer.write(b"\0")
        await writer.drain()
        async with asyncio.timeout(2.5):
            self.assertEqual(await reader.read(), b"")
        writer.close()

    async def test_concurrency_is_bounded_and_oversize_input_dropped(self):
        local, external = await self.fixture(delay=1), await self.fixture()
        proxy = await self.proxy(local, external)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            for _ in range(100):
                sock.sendto(dns.question("node.fixture.meshway.test", 1), proxy.endpoint)
            await asyncio.sleep(0.1)
            self.assertLessEqual(len(proxy.tasks), 32)
            self.assertLessEqual(len(local.requests), 32)
        await proxy.close()
        self.proxies.remove(proxy)
        proxy = await self.proxy(local, external)
        before = len(local.requests)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.sendto(b"x" * 4097, proxy.endpoint)
            await asyncio.sleep(0.02)
        self.assertEqual(len(local.requests), before)

    async def test_tcp_port_conflict_rolls_back_udp_before_any_file_change(self):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as taken:
            taken.bind(("127.0.0.1", 0))
            taken.listen()
            endpoint = taken.getsockname()
            local, external = await self.fixture(), await self.fixture()
            proxy = dns.Proxy(dns.Router("fixture.meshway.test", [], PREFIXES, local.endpoint, [external.endpoint]))
            with self.assertRaises(OSError):
                await proxy.bind(endpoint)
            await asyncio.sleep(0)
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
                udp.bind(endpoint)

    async def test_run_sigterm_restoration_busy_run_and_disabled_readiness(self):
        local = await self.fixture(rcode=5)  # A disabled but reachable service.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            target, private = root / "resolv.conf", root / "private"
            target.write_bytes(ORIGINAL)
            target.chmod(0o640)
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as reserve:
                reserve.bind(("127.0.0.1", 0))
                listen = reserve.getsockname()
            # Test-only numeric endpoint injection and container-guard bypass;
            # the public CLI does not expose these test accommodations.
            script = """
import os, resolver
resolver.container_guard = lambda args: None
settings = resolver.run_settings
def ephemeral_settings(args):
    zone, listen, nameservice, prefixes = settings(args)
    return zone, ('127.0.0.1', int(os.environ['LISTEN_PORT'])), nameservice, prefixes
resolver.run_settings = ephemeral_settings
raise SystemExit(resolver.main())
"""
            command = [sys.executable, "-c", script, "run", "--zone", "fixture.meshway.test",
                       "--mesh-ipv4-prefix", "100.96.0.0/16", "--mesh-ipv6-prefix", "fd96::/64",
                       "--state-dir", str(private), "--resolv-conf", str(target),
                       "--nameservice", f"127.0.0.1:{local.endpoint[1]}", "--allow-container-resolver"]
            env = {**os.environ, "LISTEN_PORT": str(listen[1])}
            child = await asyncio.create_subprocess_exec(*command, cwd=HERE, env=env,
                                                        stdout=asyncio.subprocess.PIPE,
                                                        stderr=asyncio.subprocess.PIPE)
            try:
                async with asyncio.timeout(3):
                    line = await child.stdout.readline()
                self.assertEqual(json.loads(line)["state"], "active")
                self.assertNotEqual(target.read_bytes(), ORIGINAL)
                second = await asyncio.create_subprocess_exec(*command, cwd=HERE, env=env,
                                                             stdout=asyncio.subprocess.PIPE,
                                                             stderr=asyncio.subprocess.PIPE)
                async with asyncio.timeout(3):
                    _, error = await second.communicate()
                self.assertEqual(second.returncode, 1)
                self.assertIn(b"busy", error)
                child.send_signal(signal.SIGTERM)
                async with asyncio.timeout(3):
                    output, error = await child.communicate()
                self.assertEqual(child.returncode, 0, error)
                self.assertEqual(json.loads(output)["state"], "restored")
                self.assertEqual(target.read_bytes(), ORIGINAL)
                self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o640)
                with state.Store(private) as store:
                    self.assertEqual(store.load()["phase"], "restored")
                for kind in (socket.SOCK_DGRAM, socket.SOCK_STREAM):
                    with socket.socket(socket.AF_INET, kind) as sock:
                        sock.bind(listen)
            finally:
                if child.returncode is None:
                    child.kill()
                    await child.wait()

    async def test_nameservice_failure_and_tcp_conflict_leave_file_and_journal_untouched(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            target, private = root / "resolv.conf", root / "private"
            target.write_bytes(ORIGINAL)
            args = resolver.parser().parse_args(["run", "--zone", "fixture.meshway.test",
                                                 "--mesh-ipv4-prefix", "100.96.0.0/16",
                                                 "--mesh-ipv6-prefix", "fd96::/64",
                                                 "--state-dir", str(private), "--resolv-conf", str(target)])
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as reserve, \
                    socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as unavailable:
                reserve.bind(("127.0.0.1", 0))
                listen = reserve.getsockname()
                unavailable.bind(("127.0.0.1", 0))
                nameservice = unavailable.getsockname()
                with patch.object(resolver, "container_guard"), \
                        patch.object(resolver, "run_settings", return_value=("fixture.meshway.test", listen, nameservice, PREFIXES)):
                    reserve.listen()
                    with self.assertRaises(OSError):
                        await resolver.run(args)
                    self.assertEqual(target.read_bytes(), ORIGINAL)
                    self.assertFalse((private / "journal.json").exists())
                    reserve.close()
                    with self.assertRaises(TimeoutError):
                        await resolver.run(args)
                    self.assertEqual(target.read_bytes(), ORIGINAL)
                    self.assertFalse((private / "journal.json").exists())
                    await asyncio.sleep(0)
                    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
                        sock.bind(listen)


class WireValidation(unittest.TestCase):
    def test_zone_ascii_punycode_and_reverse_validation(self):
        self.assertEqual(dns.zone_name("XN--FIQS8S.Meshway.Test."), "xn--fiqs8s.meshway.test")
        for invalid in ("中国.test", "-bad.test", "bad-.test", "bad..test", "a.test..", "a" * 64 + ".test", "bad_test"):
            with self.assertRaises(ValueError):
                dns.zone_name(invalid)
        for value in ("100.96.0.2", "fd96::12ab"):
            ip = ipaddress.ip_address(value)
            labels = dns.parse(dns.question(ip.reverse_pointer)).question[0]
            self.assertEqual(dns.reverse_address(labels), ip)
        for invalid in ("01.0.96.100.in-addr.arpa", "256.0.96.100.in-addr.arpa",
                        "1.0.96.in-addr.arpa", "g." + ".".join(["0"] * 31) + ".ip6.arpa"):
            self.assertIsNone(dns.reverse_address(tuple(p.encode() for p in invalid.split("."))))

    def test_compression_and_malformed_record_rejected(self):
        header = struct.pack("!6H", 1, 0x100, 1, 0, 0, 0)
        with self.assertRaises(dns.DNSInvalid):
            dns.parse(header + b"\xc0\x0c\0\1\0\1")
        data = answer(dns.question("a.test", 1))
        for invalid in (data[:-1], data + b"x", data[:12], b"x" * 4097):
            with self.assertRaises(dns.DNSInvalid):
                dns.parse(invalid)


class FileTransaction(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.path = self.root / "resolv.conf"
        self.path.write_bytes(ORIGINAL)
        self.path.chmod(0o640)
        self.state_path = self.root / "private"

    def tearDown(self):
        self.temp.cleanup()

    def apply(self, store, target, zone="fixture.meshway.test"):
        journal, previous = target.prepare(store, zone, ("127.0.0.1", 53), ("127.0.0.1", 5353), PREFIXES)
        target.apply(store, journal, previous)
        return journal

    def test_bytes_mode_inode_restore_reapply_previous_zone_and_idempotent_down(self):
        original_inode = self.path.stat().st_ino
        with state.Store(self.state_path) as store, state.ResolverFile(self.path) as target:
            store.lock()
            journal = self.apply(store, target)
            self.assertEqual(stat.S_IMODE(self.state_path.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE((self.state_path / "journal.json").stat().st_mode), 0o600)
            self.assertIn(b"search fixture.meshway.test\n", self.path.read_bytes())
            self.assertNotIn(b"old.test", self.path.read_bytes())
            self.assertIn(b"options ndots:0 timeout:1 attempts:2", self.path.read_bytes())
            self.assertEqual(self.path.stat().st_ino, original_inode)
            target.restore(store, journal)
            self.assertEqual(self.path.read_bytes(), ORIGINAL)
            self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o640)
        with state.Store(self.state_path) as store, state.ResolverFile(self.path) as target:
            store.lock()
            journal = self.apply(store, target, "new.meshway.test")
            self.assertEqual(journal["zones"], ["fixture.meshway.test", "new.meshway.test"])
            target.restore(store, journal, forget=True)
        args = resolver.parser().parse_args(["down", "--state-dir", str(self.state_path),
                                             "--resolv-conf", str(self.path), "--allow-container-resolver"])
        with patch.object(resolver, "container_guard"), contextlib.redirect_stdout(io.StringIO()):
            resolver.down(args)
            resolver.down(args)
        self.assertEqual(self.path.read_bytes(), ORIGINAL)

    def test_third_party_content_inode_and_symlink_conflicts_never_overwrite(self):
        with state.Store(self.state_path) as store, state.ResolverFile(self.path) as target:
            store.lock()
            journal = self.apply(store, target)
            self.path.write_bytes(b"nameserver 192.0.2.5\n")
            with self.assertRaises(state.Conflict):
                target.restore(store, journal, forget=True)
            self.assertEqual(self.path.read_bytes(), b"nameserver 192.0.2.5\n")
            self.assertIsNotNone(store.load())
            self.path.unlink()
            self.path.write_bytes(state.decoded(journal["written"]))
            with self.assertRaises(state.Conflict):
                target.restore(store, journal)
            self.path.unlink()
            other = self.root / "other"
            other.write_bytes(b"do not touch")
            self.path.symlink_to(other)
            with self.assertRaises(OSError):
                target.restore(store, journal)
            self.assertEqual(other.read_bytes(), b"do not touch")
        with self.assertRaises(OSError):
            state.ResolverFile(self.path)

    def test_partial_write_failure_restores_original_and_removes_new_journal(self):
        real_write, calls = os.write, 0

        with state.Store(self.state_path) as store, state.ResolverFile(self.path) as target:
            store.lock()
            journal, previous = target.prepare(store, "fixture.meshway.test", ("127.0.0.1", 53),
                                               ("127.0.0.1", 5353), PREFIXES)

            def fail_once(fd, data):
                nonlocal calls
                if fd == target.fd:
                    calls += 1
                    if calls == 1:
                        return real_write(fd, data[:7])
                    if calls == 2:
                        raise OSError("injected write failure")
                return real_write(fd, data)

            with patch.object(state.os, "write", side_effect=fail_once):
                with self.assertRaises(OSError):
                    target.apply(store, journal, previous)
            self.assertEqual(self.path.read_bytes(), ORIGINAL)
            self.assertIsNone(store.load())

    def test_fsync_failure_after_install_rolls_back_and_retains_prior_history(self):
        with state.Store(self.state_path) as store, state.ResolverFile(self.path) as target:
            store.lock()
            journal = self.apply(store, target)
            target.restore(store, journal)
        with state.Store(self.state_path) as store, state.ResolverFile(self.path) as target:
            store.lock()
            journal, previous = target.prepare(store, "new.meshway.test", ("127.0.0.1", 53),
                                               ("127.0.0.1", 5353), PREFIXES)
            real_fsync, failed = os.fsync, False

            def fail_once(fd):
                nonlocal failed
                if fd == target.fd and not failed:
                    failed = True
                    raise OSError("injected fsync failure")
                real_fsync(fd)

            with patch.object(state.os, "fsync", side_effect=fail_once):
                with self.assertRaises(OSError):
                    target.apply(store, journal, previous)
            self.assertEqual(self.path.read_bytes(), ORIGINAL)
            self.assertEqual(store.load()["zones"], ["fixture.meshway.test"])

    def test_original_upstream_loop_rejection_preserves_docker_stub(self):
        upstreams, _, _ = state.original_config(ORIGINAL, ("127.0.0.1", 53), ("127.0.0.1", 5353))
        self.assertIn(("127.0.0.11", 53), upstreams)
        self.assertIn(("2001:db8::53", 53), upstreams)
        for ip in ("127.0.0.1", "::1", "127.0.0.2", "::ffff:127.0.0.1", "0.0.0.0", "::", "224.0.0.1"):
            with self.assertRaises(state.Conflict):
                state.original_config(f"nameserver {ip}\n".encode(), ("127.0.0.1", 53), ("127.0.0.1", 5353))
        with self.assertRaises(state.Conflict):
            state.original_config(b"nameserver 127.0.0.11\n", ("127.0.0.1", 53), ("127.0.0.11", 53))

    def test_prewrite_edit_aborts_without_overwrite_or_journal_loss(self):
        with state.Store(self.state_path) as store, state.ResolverFile(self.path) as target:
            store.lock()
            journal, previous = target.prepare(store, "fixture.meshway.test", ("127.0.0.1", 53),
                                               ("127.0.0.1", 5353), PREFIXES)
            self.path.write_bytes(b"admin edit\n")
            with self.assertRaises(state.Conflict):
                target.apply(store, journal, previous)
            self.assertEqual(self.path.read_bytes(), b"admin edit\n")
            self.assertIsNotNone(store.load())

    def test_sigkill_lock_busy_stale_status_and_down_recovery(self):
        script = """
import ipaddress, os, time
from state import Store, ResolverFile
with Store(os.environ['STATE']) as store, ResolverFile(os.environ['TARGET']) as target:
    store.lock()
    j, previous = target.prepare(store, 'fixture.meshway.test', ('127.0.0.1', 9), ('127.0.0.1', 5353), [ipaddress.ip_network('100.96.0.0/16'), ipaddress.ip_network('fd96::/64')])
    target.apply(store, j, previous)
    print('ready', flush=True)
    time.sleep(60)
"""
        child = subprocess.Popen([sys.executable, "-c", script], cwd=HERE,
                                 env={**os.environ, "STATE": str(self.state_path), "TARGET": str(self.path)},
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), "ready")
            with state.Store(self.state_path) as store:
                with self.assertRaises(state.Conflict):
                    store.lock()
            child.send_signal(signal.SIGKILL)
            child.wait(timeout=3)
            output = io.StringIO()
            args = resolver.parser().parse_args(["status", "--state-dir", str(self.state_path)])
            with contextlib.redirect_stdout(output):
                resolver.status(args)
            result = json.loads(output.getvalue())
            self.assertEqual(result["state"], "recovery-required")
            self.assertFalse(result["pid_alive"])
            self.assertFalse(result["listener_udp"])
            self.assertFalse(result["listener_tcp"])
            with state.Store(self.state_path) as store, state.ResolverFile(self.path) as target:
                store.lock()
                target.restore(store, store.load(), forget=True)
                self.assertIsNone(store.load())
            self.assertEqual(self.path.read_bytes(), ORIGINAL)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            child.stdout.close()
            child.stderr.close()

    def test_guards_and_status_absent_have_no_host_mutation(self):
        args = resolver.parser().parse_args(["run", "--zone", "fixture.meshway.test",
                                             "--mesh-ipv4-prefix", "100.96.0.0/16",
                                             "--mesh-ipv6-prefix", "fd96::/64", "--state-dir", str(self.state_path)])
        with self.assertRaises(state.Conflict):
            resolver.container_guard(args)
        args.allow_container_resolver = True
        with patch.object(resolver.sys, "platform", "darwin"), self.assertRaises(state.Conflict):
            resolver.container_guard(args)
        args.listen = "0.0.0.0:53"
        with self.assertRaises(state.Conflict):
            resolver.run_settings(args)
        args = resolver.parser().parse_args(["status", "--state-dir", str(self.state_path)])
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            resolver.status(args)
        self.assertEqual(json.loads(output.getvalue())["state"], "absent")
        self.assertFalse(self.state_path.exists())


if __name__ == "__main__":
    unittest.main()
