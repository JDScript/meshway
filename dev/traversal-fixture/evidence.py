"""Bounded, fixed-format capture inspection, never a Mesh packet implementation.

Only Ethernet/IPv4/UDP/TCP headers and public UDP framing discriminators are inspected.
Authentication, offer authority, encryption and complete-path proof remain the
production implementations' responsibility and are checked by the Rust actor.
"""
import collections
import ipaddress
import json
import re
import shlex
import struct
from typing import NamedTuple

LIMIT = 8 * 1024 * 1024


class Packet(NamedTuple):
    source: str
    source_port: int
    destination: str
    destination_port: int
    kind: str
    timestamp_us: int
    offer_digest: str | None = None


COORDINATION_KINDS = {0: "coordinationPunch", 1: "coordinationInit", 2: "coordinationChallenge",
                      3: "coordinationAck", 4: "coordinationReady"}


def packets(path):
    if path.stat().st_size > LIMIT:
        raise RuntimeError("Router capture exceeded its fixed evidence bound")
    data = path.read_bytes()
    if len(data) < 24 or data[:4] not in (b"\xd4\xc3\xb2\xa1", b"\xa1\xb2\xc3\xd4"):
        raise RuntimeError("Expected classic bounded microsecond pcap")
    order = "<" if data[:4] == b"\xd4\xc3\xb2\xa1" else ">"
    major, minor, _, _, snaplen, link = struct.unpack(order + "HHIIII", data[4:24])
    if (major, minor, link) != (2, 4, 1) or snaplen != 128:
        raise RuntimeError("Unexpected capture schema/link/snaplen")
    offset = 24
    count = 0
    while offset < len(data):
        if offset + 16 > len(data):
            raise RuntimeError("Truncated capture record")
        seconds, micros, size, original = struct.unpack_from(order + "IIII", data, offset)
        offset += 16
        if size > snaplen or size > original or offset + size > len(data) or micros >= 1_000_000:
            raise RuntimeError("Invalid capture record bounds")
        frame = data[offset:offset + size]
        offset += size
        count += 1
        if count > 40000:
            raise RuntimeError("Capture packet count exceeded")
        if len(frame) < 14 or frame[12:14] != b"\x08\x00":
            raise RuntimeError("Non-Ethernet/IPv4 packet in IPv4-only proof")
        packet = frame[14:]
        if len(packet) < 28 or packet[0] >> 4 != 4 or packet[9] not in (6, 17):
            raise RuntimeError("Unexpected IP protocol in traversal capture")
        header = (packet[0] & 15) * 4
        if header < 20 or len(packet) < header + 8 or int.from_bytes(packet[6:8], "big") & 0x3fff:
            raise RuntimeError("Malformed or fragmented fixture packet")
        source = str(ipaddress.ip_address(packet[12:16]))
        destination = str(ipaddress.ip_address(packet[16:20]))
        if packet[9] == 6:
            if len(packet) < header + 20:
                raise RuntimeError("Truncated TCP header")
            sport, dport = struct.unpack_from("!HH", packet, header)
            tcp_header = (packet[header + 12] >> 4) * 4
            length = int.from_bytes(packet[2:4], "big") - header
            if tcp_header < 20 or tcp_header > length or len(packet) < header + tcp_header:
                raise RuntimeError("Invalid TCP data offset/length")
            flags = packet[header + 13]
            kind = ("tcpReset" if flags & 4 else "tcpSynAck" if flags & 2 and flags & 16 else "tcpSyn" if flags & 2
                    else "tcpData" if length > tcp_header else "tcpAck")
            yield Packet(source, sport, destination, dport, kind, seconds * 1_000_000 + micros)
            continue
        sport, dport, length, _ = struct.unpack_from("!HHHH", packet, header)
        if length < 8 or int.from_bytes(packet[2:4], "big") != header + length:
            raise RuntimeError("Invalid UDP/IP lengths")
        payload = packet[header + 8:]
        offer_digest = None
        if len(payload) >= 20 and payload[4:8] == b"\x21\x12\xa4\x42" and payload[0] & 0xc0 == 0:
            kind = {b"\x00\x01": "stunRequest", b"\x01\x01": "stunResponse"}.get(payload[:2], "stunOther")
        elif payload.startswith(b"MWTRV1\0\0"):
            # The signed exchange is INSIDE the offer-bound envelope, not an
            # MWADJ prefix. Only the public 41-byte header is inspected here.
            if len(payload) < 41 or payload[40] not in COORDINATION_KINDS:
                raise RuntimeError("Truncated or unknown traversal envelope header")
            offer_digest = payload[8:40].hex()
            kind = COORDINATION_KINDS[payload[40]]
        elif payload.startswith(b"MWADJv1\0") and len(payload) >= 11:
            kind = {1: "adjacencyInit", 2: "adjacencyChallenge", 3: "adjacencyAck",
                    4: "adjacencyReady", 5: "adjacencyPing", 6: "adjacencyPong"}.get(payload[8], "other")
        elif payload.startswith(b"MWCIR\0\0\x04"):
            kind = "meshCiphertext"
        else:
            kind = "other"
        yield Packet(source, sport, destination, dport, kind, seconds * 1_000_000 + micros, offer_digest)


def bounded_json(path):
    if path.stat().st_size > LIMIT:
        raise RuntimeError("Fallback JSON exceeds evidence bound")
    return json.loads(path.read_text())


def forwarding_dnat(firewall, baseline):
    """Exclude only unchanged Docker embedded DNS in the local OUTPUT path."""
    def rules(text):
        result = []
        for line in text.splitlines():
            match = re.fullmatch(r"\[(\d+):(\d+)\] (.+)", line)
            if match:
                result.append((int(match[1]), shlex.split(match[3])))
            elif "-j DNAT" in line:
                raise RuntimeError("DNAT lacks an actual kernel counter record")
        return result

    current = rules(firewall)
    previous = rules(baseline)
    hook = shlex.split("-A OUTPUT -d 127.0.0.11/32 -j DOCKER_OUTPUT")
    local_only = all([tokens for _, tokens in entries if "DOCKER_OUTPUT" in tokens[2:]] == [hook]
                     for entries in (current, previous))
    result = []
    for count, tokens in current:
        options = dict(zip(tokens[::2], tokens[1::2]))
        if options.get("-j") != "DNAT":
            continue
        dns = re.fullmatch(r"-A DOCKER_OUTPUT -d 127\.0\.0\.11/32 -p (tcp|udp) -m \1 --dport 53 "
                           r"-j DNAT --to-destination 127\.0\.0\.11:([0-9]+)", " ".join(tokens))
        if (local_only and dns and 0 < int(dns[2]) <= 65535
                and any(tokens == old for _, old in previous)):
            continue
        result.append((count, options))
    return result


def handshake(records, local, target, public_local, offers):
    """Correlate public headers with Rust-verified real HTTP/State offers.

    Four signed phases must have the same digest, exact endpoint pair, correct
    opposing directions and causal order, followed by ciphertext both ways.
    Signatures and authority are checked only by the production Node and the
    typed Rust audit, never reconstructed from untrusted packet bodies here.
    """
    pair = {f"{public_local}:35000", f"{target}:35000"}
    approved = {offer["digest"] for offer in offers
                if {offer["local"]["address"], offer["peer"]["address"]} == pair}
    exact = [packet for packet in records if packet.source_port == packet.destination_port == 35000
             and (packet.source, packet.destination) in ((local, target), (target, local))]
    for digest in sorted(approved):
        if re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            raise RuntimeError("Invalid typed-audit offer digest")
        bound = [packet for packet in exact if packet.offer_digest == digest]
        for initiator, responder in ((local, target), (target, local)):
            phases = {}
            previous = 0
            for kind, source, destination in (("coordinationInit", initiator, responder),
                                              ("coordinationChallenge", responder, initiator),
                                              ("coordinationAck", initiator, responder),
                                              ("coordinationReady", responder, initiator)):
                observed = [packet.timestamp_us for packet in bound
                            if (packet.kind, packet.source, packet.destination) == (kind, source, destination)
                            and packet.timestamp_us >= previous]
                if not observed:
                    break
                previous = min(observed)
                phases[kind] = previous
            if len(phases) != 4:
                continue
            ciphertext = {}
            for direction, source, destination in (("outbound", local, target), ("inbound", target, local)):
                observed = [packet.timestamp_us for packet in exact
                            if (packet.kind, packet.source, packet.destination) == ("meshCiphertext", source, destination)
                            and packet.timestamp_us >= previous]
                if observed:
                    ciphertext[direction] = min(observed)
            if len(ciphertext) == 2:
                return {"offerDigest": digest, "initiator": initiator, "responder": responder,
                        "phaseUnixUs": phases, "ciphertextUnixUs": ciphertext,
                        "punchPackets": sum(packet.kind == "coordinationPunch" for packet in bound)}
    raise RuntimeError("Missing complete same-offer bilateral traversal handshake before ciphertext")


def verify(report, actors, routers, *, fallback=False, short=False):
    evidence = {"outerFamily": "ipv4", "natModel": "linux-conntrack-port-preserving-snat",
                "path": ["a", "r", "b"], "routers": {}, "fallback": None}
    offers = bounded_json(report / "controller/traversal-verified-offers.json")
    blocked_us = None
    enabled_us = None
    enable_started_us = None
    if fallback:
        activation = bounded_json(report / "router-r/fallback.json")
        enable_started_us = activation["enableStartedAtMs"] * 1000
        enabled_us = activation["enabledAtMs"] * 1000
        blocked_us = bounded_json(report / "router-a/fallback.json")["blockedAtMs"] * 1000
        if not 0 < enable_started_us <= enabled_us <= blocked_us:
            raise RuntimeError("TCP DNAT was not enabled before the A-R UDP block")
        evidence["fallback"] = {"tcpEnabledAtMs": enabled_us // 1000, "udpBlockedAtMs": blocked_us // 1000,
                                "path": ["a", "r", "b"], "carriers": {"a-r": "native_tcp", "r-b": "native_udp"}}
    for role, router in routers.items():
        directory = report / ("router-" + role)
        sides = {}
        handshakes = {}
        for side, local in (("private", actors[role]), ("wan", router["wan"])):
            capture_log = directory / (side + "-capture.stderr")
            if capture_log.stat().st_size > 64 * 1024:
                raise RuntimeError("Capture diagnostics exceeded bound")
            drops = re.findall(r"^(\d+) packets dropped by kernel$", capture_log.read_text(), re.MULTILINE)
            if drops != ["0"]:
                raise RuntimeError("Packet capture did not stop cleanly with zero kernel drops")
            counts = collections.Counter()
            initial = collections.Counter()
            after = collections.Counter()
            intervals = {}
            initial_records = []
            for packet in packets(directory / (side + ".pcap")):
                source, sport, destination, dport, kind, timestamp, _ = packet
                key = (source, sport, destination, dport, kind)
                counts[key] += 1
                if blocked_us is None or timestamp < blocked_us:
                    initial[key] += 1
                    initial_records.append(packet)
                else:
                    after[key] += 1
                if enable_started_us is not None and timestamp < enable_started_us and kind in ("tcpData", "tcpSynAck"):
                    raise RuntimeError("TCP fallback channel was available before the explicit DNAT transition")
                first, last = intervals.get(key, (timestamp, timestamp))
                intervals[key] = min(first, timestamp), max(last, timestamp)
            targets = [routers["r" if role == "a" else "a"]["wan"]]
            if role == "r":
                targets.append(actors["b"])
            required = [(local, 35000, actors["controller"], 3478, "stunRequest"),
                        (actors["controller"], 3478, local, 35000, "stunResponse")]
            handshakes[side] = {target: handshake(initial_records, local, target, router["wan"], offers)
                                for target in targets}
            if any(initial[key] == 0 for key in required):
                raise RuntimeError(f"Missing real same-port STUN/punch/handshake/Mesh packets: {role}/{side}")
            if side == "wan" and any(actors[role] in (key[0], key[2]) for key in counts):
                raise RuntimeError("Private actor address escaped SNAT")
            if fallback:
                client = local if role == "a" else routers["a"]["wan"]
                server = routers["r"]["wan"] if role == "a" else local
                ports = {sport for (src, sport, dst, dport, kind), count in after.items()
                         if count and (src, dst, dport, kind) == (client, server, 35001, "tcpData")}
                if not any(after[(server, 35001, client, port, "tcpData")] > 0 for port in ports):
                    raise RuntimeError(f"No bidirectional same-stream TCP data after block: {role}/{side}")
                if role == "r" and any(after[key] == 0 for key in (
                        (local, 35000, actors["b"], 35000, "meshCiphertext"),
                        (actors["b"], 35000, local, 35000, "meshCiphertext"))):
                    raise RuntimeError("R-B stopped using actual Native UDP after A-R fallback")
            sides[side] = [{"source": source, "sourcePort": sport, "destination": destination,
                            "destinationPort": dport, "kind": kind, "packets": count,
                            "firstUnixUs": intervals[key][0], "lastUnixUs": intervals[key][1]}
                           for key, count in sorted(counts.items())
                           for source, sport, destination, dport, kind in [key]]
        # These are actual kernel snapshots taken before the daemon's offline
        # teardown. Do not infer mapping state from successful application echo.
        for name in ("conntrack-first.txt", "conntrack-traffic.txt", "firewall-before.txt", "firewall-ready.txt", "firewall-traffic.txt"):
            path = directory / name
            if not path.is_file() or path.stat().st_size > LIMIT or not path.read_text().strip():
                raise RuntimeError("Missing bounded live router kernel evidence: " + name)
        conntrack = (directory / "conntrack-traffic.txt").read_text()
        if f"src={actors[role]} " not in conntrack or f"dst={router['wan']} " not in conntrack or "sport=35000 " not in conntrack:
            raise RuntimeError("Live conntrack does not show the private/public UDP mapping")
        firewall = (directory / "firewall-traffic.txt").read_text()
        if not re.search(r"^:FORWARD DROP \[\d+:\d+\]$", firewall, re.MULTILINE):
            raise RuntimeError("Router does not enforce default-deny forwarding")
        counters = {}
        fallback_counters = {}
        baseline = (directory / "firewall-before.txt").read_text()
        if forwarding_dnat((directory / "firewall-ready.txt").read_text(), baseline):
            raise RuntimeError("Forwarding DNAT existed before the traversal proof")
        for count, options in forwarding_dnat(firewall, baseline):
            if (not fallback or role != "r" or options.get("-A") != "PREROUTING"
                    or options.get("-p") != "tcp" or options.get("--dport") != "35001"
                    or options.get("--to-destination") != actors["r"] + ":35001"
                    or options.get("-d") != routers["r"]["wan"] + "/32"):
                raise RuntimeError("Unexpected DNAT (UDP forwarding is never permitted)")
            fallback_counters["tcpDnatPackets"] = count
        for line in firewall.splitlines():
            match = re.fullmatch(r"\[(\d+):(\d+)\] (.+)", line)
            if not match:
                if "-j DNAT" in line:
                    raise RuntimeError("DNAT lacks an actual kernel counter record")
                continue
            tokens = shlex.split(match[3])
            options = dict(zip(tokens[::2], tokens[1::2]))
            if fallback and role == "a" and options.get("-A") == "FORWARD" and options.get("-p") == "udp" and options.get("-j") == "DROP":
                pair = (options.get("-s"), options.get("-d"))
                if pair == (actors["a"] + "/32", routers["r"]["wan"] + "/32"):
                    fallback_counters["udpOutboundDrops"] = int(match[1])
                elif pair == (routers["r"]["wan"] + "/32", actors["a"] + "/32"):
                    fallback_counters["udpInboundDrops"] = int(match[1])
                else:
                    raise RuntimeError("Fallback UDP block expanded beyond the A-R IP pair")
            if options.get("-j") == "SNAT" and options.get("-s") == actors[role] + "/32" and options.get("--to-source") == router["wan"]:
                counters["snatPackets"] = int(match[1])
            if (options.get("-A") == "FORWARD" and options.get("-j") == "ACCEPT"
                    and options.get("-d") == actors[role] + "/32"
                    and set(options.get("--ctstate", "").split(",")) == {"ESTABLISHED", "RELATED"}):
                counters["establishedReturnPackets"] = int(match[1])
        if set(counters) != {"snatPackets", "establishedReturnPackets"} or not all(counters.values()):
            raise RuntimeError("Missing actual SNAT/established-return firewall counters")
        if fallback:
            if forwarding_dnat((directory / "firewall-before-fallback.txt").read_text(), baseline):
                raise RuntimeError("Fallback DNAT existed during initial punched UDP proof")
            expected = {"udpOutboundDrops", "udpInboundDrops"} if role == "a" else {"tcpDnatPackets"}
            if set(fallback_counters) != expected or sum(fallback_counters.values()) == 0:
                raise RuntimeError("Missing actual fallback DROP/DNAT counters")
            tcp = directory / "conntrack-tcp-traffic.txt"
            if tcp.stat().st_size > LIMIT or not any("ESTABLISHED" in line and "dport=35001 " in line
                                                    for line in tcp.read_text().splitlines()):
                raise RuntimeError("Missing live established TCP fallback conntrack entry")
        evidence["routers"][role] = {"private": router["private"], "wan": router["wan"],
                                     "firewallCounters": counters, "fallbackCounters": fallback_counters if fallback else None,
                                     "capture": sides, "handshakes": handshakes}
    if fallback:
        before = bounded_json(report / "a/fallback-before.json")
        after = bounded_json(report / "a/fallback-after.json")
        if (not (5_000 if short else 30_000) <= blocked_us // 1000 - before["observedAtMs"] < 100_000
                or after["observedAtMs"] <= blocked_us // 1000
                or before["packet"]["sockets"] != after["packet"]["sockets"]
                or any(after["packet"]["counts"][kind] <= before["packet"]["counts"][kind] for kind in ("udp4", "udp6", "tcp4", "tcp6"))
                or after["packet"]["lastEchoSequence"] <= before["packet"]["lastEchoSequence"]):
            raise RuntimeError("Fallback lacks uninterrupted original application socket/echo evidence")
        continuity = bounded_json(report / "a/fallback-circuits.json")
        if continuity.get("passed") is not True or len(continuity.get("tcpCircuits", [])) < 2:
            raise RuntimeError("Fallback lacks production circuit continuity evidence")
        evidence["fallback"].update(applicationBefore=before, applicationAfter=after, circuits=continuity)
    (report / "traversal-packets.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return evidence
