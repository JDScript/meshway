#!/usr/bin/env python3
"""Reproducible arithmetic over raw crypto/L0 reports (never inferred throughput)."""
import argparse
from collections import Counter
import json
from pathlib import Path

import circuit


def routes(environment, nodes):
    rows = {(r["operation"], r["bytes"]): r["cpuNsPerOp"]["median"] / 1000
            for r in environment["aggregates"]}
    output = []
    for size in (52, 1280):
        get = lambda name: rows[name, size]
        old_seal = get("chacha0.10.seal.inplace.new")
        old_open = get("chacha0.10.open.inplace.new.copy")
        old_mac = get("hmac0.12.sha0.10.v5.new")
        for aead in ("chacha0.11", "ring"):
            seal_saved = old_seal - get(aead + ".seal.inplace.new")
            open_saved = old_open - get(aead + ".open.inplace.new.copy")
            for route, mac in (
                ("i-new-hmac", "hmac0.13.sha0.11.v5.new"),
                ("i-ring-hmac", "ring.hmac.v5.new"),
                ("ii-old-prekey", "hmac0.12.sha0.10.v5.clone"),
                ("ii-new-prekey", "hmac0.13.sha0.11.v5.clone"),
                ("ii-ring-prekey", "ring.hmac.v5.prekey"),
                ("iii-aead-tag", aead + ".aad_tag.v5.new"),
                ("iii-blake3", "blake3.keyed.v5.tag16"),
            ):
                mac_saved = old_mac - get(mac)
                output.append(dict(bytes=size, nodes=nodes, route=route, aead=aead, mac=mac,
                                   originSavedUs=seal_saved + mac_saved,
                                   endpointSavedUs=open_saved + mac_saved,
                                   relaySavedUs=2 * mac_saved,
                                   pathSavedUs=seal_saved + open_saved + 2 * (nodes - 1) * mac_saved,
                                   oldPrimitivePathUs=old_seal + old_open + 2 * (nodes - 1) * old_mac))
    return output


def category(stack):
    """Disjoint ancestry attribution; nested codec/buffer costs keep their own category."""
    joined = "\n".join(stack)
    leaf = stack[0]
    if (any(s in leaf for s in ("memcpy", "memmove", "memset", "bzero"))
            or any(s in joined for s in ("::buffer::PacketBuffer::", "::buffer::PacketPool::", "::buffer::PacketBuffer as"))):
        return "buffer-copy"
    if "::codec::" in joined:
        return "frame-codec"
    if "::session::authenticate" in joined or "::session::MacKey::" in joined or "hmac::" in joined:
        return "hmac"
    if any(s in joined for s in ("::noise::Transport::encrypt", "::noise::Transport::decrypt", "chacha20poly1305::", "chacha20::", "poly1305::")):
        return "aead"
    if "sha2::" in joined:
        return "sha256-other"
    if "meshway_authority::" in joined or "::Guard::" in joined or "CancellationToken" in joined or "::Circuit::check_at" in joined:
        return "guard"
    if "::packet::" in joined:
        return "packet-parse"
    if any(s in joined for s in ("::collections::btree", "::flow::", "::engine::", "::admission::", "meshway_policy::", "::Replay::")):
        return "flow-map-replay-policy"
    return "other"


def sampled(profile, symbols, budget_us):
    """Read samply's processed Firefox profile plus its presymbolicated sidecar."""
    symbol_strings = symbols["string_table"]
    normalize = lambda value: value.replace("-", "").upper()
    libraries = {(lib["debug_name"], normalize(lib["debug_id"])): lib for lib in symbols["data"]}
    lookups = []
    for lib in profile["libs"]:
        data = libraries.get((lib["debugName"], normalize(lib["breakpadId"])))
        lookups.append({address: data["symbol_table"][index] for address, index in data["known_addresses"]} if data else {})
    own, groups, role_groups = Counter(), Counter(), {}
    samples_seen = 0
    unresolved = 0
    for thread in profile["threads"]:
        if "meshway-circuit" not in thread["name"] and not thread.get("isMainThread"):
            continue
        frames = thread["frameTable"]
        functions = thread["funcTable"]
        resources = thread["resourceTable"]
        strings = thread["stringArray"]
        cache = {}
        def frame_names(frame):
            if frame in cache:
                return cache[frame]
            func = frames["func"][frame]
            resource = functions["resource"][func]
            lib = resources["lib"][resource] if resource >= 0 else None
            info = lookups[lib].get(frames["address"][frame]) if lib is not None else None
            if info:
                names = [symbol_strings[f["function"]] for f in (info.get("frames") or []) if f.get("function") is not None]
                if not names:
                    names = [symbol_strings[info["symbol"]]]
            else:
                names = [strings[functions["name"][func]]]
            cache[frame] = names
            return names
        table = thread["stackTable"]
        samples = thread["samples"]
        weights = samples.get("threadCPUDelta") or [1] * len(samples["time"])
        start = min(samples["time"], default=0)
        end = max(samples["time"], default=0)
        # Trim startup/handshake and shutdown independently of profiler process startup.
        for index, (time, stack_id) in enumerate(zip(samples["time"], samples["stack"])):
            if stack_id is None or not start + 200 <= time <= end - 100:
                continue
            stack = []
            while stack_id is not None:
                stack.extend(frame_names(table["frame"][stack_id]))
                stack_id = table["prefix"][stack_id]
            if not stack:
                continue
            weight = weights[index]
            if weight is None or weight <= 0:
                continue
            samples_seen += 1
            unresolved += int(stack[0].startswith("0x"))
            own[stack[0]] += weight
            group = category(stack)
            groups[group] += weight
            role = ("send" if any("SecureNode::send" in s for s in stack) else
                    "receive" if any("SecureNode::receive" in s for s in stack) else "harness")
            role_groups.setdefault(role, Counter())[group] += weight
    total = sum(own.values())
    if not total:
        raise ValueError("no usable main-thread CPU samples")
    scale = lambda counts: {name: dict(percent=value / total * 100, cpuUsPerPacket=value / total * budget_us)
                            for name, value in counts.items()}
    return dict(samples=samples_seen, unresolvedLeafSamples=unresolved, budgetUs=budget_us,
                weighting="threadCPUDelta where available, otherwise sample count; first 200ms/last 100ms trimmed",
                groups=scale(groups), roles={role: scale(counts) for role, counts in role_groups.items()},
                functions=[dict(function=name, **values) for name, values in scale(dict(own.most_common())).items()],
                top15=[dict(function=name, **values) for name, values in scale(dict(own.most_common(15))).items()])


def summary(report):
    lines = ["# L0 CPU attribution and measured primitive deltas", "",
             "Native attribution = sampled CPU weight × uninstrumented harness CPU/packet; statistical estimate, not exact invocation timing.",
             "Logging scopes are observer-affected; inclusive rows overlap, self estimates are not a partition of uninstrumented CPU.",
             "Savings are primitive substitution arithmetic, not measured optimized L0 or I/O throughput.", "",
             "Scope ownership: crypto.encrypt/decrypt → pipeline::seal/open → noise::Transport;",
             "crypto.append_mac/verify_hop_mac → session::MacKey → authenticate (includes Frame::validate + fresh HMAC key schedule);",
             "codec.encode/decode → Frame::encode_owned/decode_owned; flow.admit → Engine::admit_inner;",
             "secure.send → prepare_send_packet; secure.prepare_receive/commit_receive → owner prepare/commit;",
             "packet.parse → Packet::parse; control.guard → ExecutionSnapshot::reading/ensure_current.", ""]
    for environment, groups in report["l0"].items():
        for group in groups:
            lines += [f"## {environment}, {group['nodes']} nodes, 1280B", "",
                      f"Uninstrumented harness CPU: {group['cpuUsPerPacket']:.3f} µs/one-way delivered packet.", "",
                      "| Role | CPU µs/call |", "| --- | ---: |"]
            for name, role in group["roles"].items():
                lines.append(f"| {name} | {role['cpuNs']['median']/1000:.3f} |")
            if group["native"]:
                native = group["native"]
                lines += ["", f"Native samples: {native['samples']}; unresolved leaves: {native['unresolvedLeafSamples']}.",
                          "", "| Category | CPU % | CPU µs/packet |", "| --- | ---: | ---: |"]
                for name, values in sorted(native["groups"].items()):
                    lines.append(f"| {name} | {values['percent']:.2f} | {values['cpuUsPerPacket']:.3f} |")
                lines += ["", "### Native top 15 self-time", "", "| Function | CPU % | CPU µs/packet |", "| --- | ---: | ---: |"]
                for row in native["top15"]:
                    lines.append(f"| `{row['function'].replace('|', '&#124;')}` | {row['percent']:.2f} | {row['cpuUsPerPacket']:.3f} |")
            on = group["instrumented"]
            lines += ["", f"Profile=128 CPU: {on['harnessCpuNsPerPacket']['median']/1000:.3f} µs/packet.",
                      "", "### Logging top 15 self-time (existing scopes)", "",
                      "| Scope | Calls/packet | Inclusive µs/packet estimate | Self µs/packet estimate |", "| --- | ---: | ---: | ---: |"]
            for row in on["profile"][:15]:
                inc = row["cpu_nsPerPacketEstimate"]
                own = row["self_cpu_nsPerPacketEstimate"]
                inc = f"{inc['median']/1000:.3f}" if inc else "unmeasured"
                own = f"{own['median']/1000:.3f}" if own else "unmeasured"
                lines.append(f"| `{row['name']}` | {row['callsPerPacket']['median']:.2f} | {inc} | {own} |")
    for environment, rows in report["routes"].items():
        lines += ["", f"## {environment}: substitution arithmetic", "",
                  "seal + open + 2 × (nodes − 1) MAC; .v5 MAC rows use IP length plus authenticated framing/tag bytes.",
                  "Positive = saved CPU. Open copies exist in both old/new microbench rows; no copy subtraction.", "",
                  "| IP bytes | Nodes | Route | AEAD | Origin saved µs | Endpoint saved µs | Each relay saved µs | Path saved µs |",
                  "| ---: | ---: | --- | --- | ---: | ---: | ---: | ---: |"]
        for row in rows:
            lines.append(f"| {row['bytes']} | {row['nodes']} | {row['route']} | {row['aead']} | "
                         f"{row['originSavedUs']:.3f} | {row['endpointSavedUs']:.3f} | {row['relaySavedUs']:.3f} | {row['pathSavedUs']:.3f} |")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crypto", type=Path, required=True)
    parser.add_argument("--host-l0", type=Path, required=True)
    parser.add_argument("--linux-l0", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    crypto = json.loads(args.crypto.read_text())
    report = {"schemaVersion": 1, "inputs": {k: str(v) for k, v in vars(args).items()}, "routes": {}, "l0": {}}
    for env in crypto["environments"]:
        if env["status"] == "ok":
            report["routes"][env["name"]] = routes(env, 2) + routes(env, 3)
    for name, path in (("host", args.host_l0), ("linux", args.linux_l0)):
        l0 = json.loads(path.read_text())
        groups = []
        for group in l0["aggregates"]:
            if group["profileRate"]:
                continue
            nodes = group["nodes"]
            budget = group["harnessCpuNsPerPacket"]["median"] / 1000
            item = dict(nodes=nodes, cpuUsPerPacket=budget, roles=group["roles"], native=None)
            if name == "host":
                profile = json.loads((path.parent / f"n{nodes}.json").read_text())
                symbols = json.loads((path.parent / f"n{nodes}.syms.json").read_text())
                item["native"] = sampled(profile, symbols, budget)
            item["instrumented"] = next(g for g in l0["aggregates"] if g["nodes"] == nodes and g["profileRate"])
            groups.append(item)
        report["l0"][name] = groups
    circuit.save(args.output, report)
    args.output.with_suffix(".md").write_text(summary(report))


if __name__ == "__main__":
    main()
