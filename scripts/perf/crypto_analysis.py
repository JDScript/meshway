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
    repeats = {}
    for sample in environment["samples"]:
        repeats.setdefault((sample["bytes"], sample["repeat"]), {})[sample["operation"]] = sample["cpuNs"] / sample["iterations"] / 1000
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
                paired = []
                for (sample_size, _), sample in repeats.items():
                    if sample_size == size:
                        paired.append(sample["chacha0.10.seal.inplace.new"] - sample[aead + ".seal.inplace.new"]
                                      + sample["chacha0.10.open.inplace.new.copy"] - sample[aead + ".open.inplace.new.copy"]
                                      + 2 * (nodes - 1) * (sample["hmac0.12.sha0.10.v5.new"] - sample[mac]))
                output.append(dict(bytes=size, nodes=nodes, route=route, aead=aead, mac=mac,
                                   originSavedUs=seal_saved + mac_saved,
                                   endpointSavedUs=open_saved + mac_saved,
                                   relaySavedUs=2 * mac_saved,
                                   pathSavedUs=seal_saved + open_saved + 2 * (nodes - 1) * mac_saved,
                                   pairedPathSavedUs=circuit.quantiles(paired),
                                   oldPrimitivePathUs=old_seal + old_open + 2 * (nodes - 1) * old_mac))
    return output


def category(stack):
    """Disjoint ancestry attribution; nested codec/buffer costs keep their own category."""
    functions = [name.replace(">", "").split("::<", 1)[0].lstrip("<") for name in stack]
    owned = lambda prefix: any(name.startswith(prefix) for name in functions)
    leaf = stack[0]
    if (any(s in leaf for s in ("memcpy", "memmove", "memset", "bzero"))
            or owned("meshway_dataplane::buffer::")):
        return "buffer-copy"
    if owned("meshway_dataplane::secure::codec::"):
        return "frame-codec"
    if owned("meshway_dataplane::secure::session::authenticate") or owned("meshway_dataplane::secure::session::MacKey") or owned("hmac::"):
        return "hmac"
    if any(owned(prefix) for prefix in ("meshway_dataplane::secure::crypto::noise::Transport", "ring::aead::", "chacha20poly1305::", "chacha20::", "poly1305::")):
        return "aead"
    if owned("sha2::"):
        return "sha256-other"
    if any(owned(prefix) for prefix in ("meshway_authority::", "meshway_dataplane::secure::Guard", "meshway_dataplane::secure::Circuit::check_at",
                                       "tokio_util::sync::cancellation_token::")):
        return "guard"
    if owned("meshway_dataplane::packet::"):
        return "packet-parse"
    if (any(owned(prefix) for prefix in ("meshway_dataplane::flow::", "meshway_dataplane::engine::", "meshway_policy::", "meshway_dataplane::secure::Replay"))
            or (owned("alloc::collections::btree::") and owned("meshway_dataplane::"))):
        return "flow-map-replay-policy"
    return "other"


def sampled(profile, symbols, budget_us):
    """Read samply's processed Firefox profile plus its presymbolicated sidecar."""
    symbol_strings = symbols["string_table"]
    def normalize(value):
        value = value.replace("-", "").upper()
        # DebugId Display omits an appendix of zero; Firefox breakpadId includes it.
        return value + "0" if len(value) == 32 else value
    libraries = {(lib["debug_name"], normalize(lib["debug_id"])): lib for lib in symbols["data"]}
    lookups = []
    for lib in profile["libs"]:
        data = libraries.get((lib["debugName"], normalize(lib["breakpadId"])))
        lookups.append({address: data["symbol_table"][index] for address, index in data["known_addresses"]} if data else {})
    own, owner, groups, role_groups, copies = Counter(), Counter(), Counter(), {}, Counter()
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
                if names[0].startswith("0x") and lib is not None:
                    names[0] += " [" + profile["libs"][lib]["name"] + "]"
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
            if any("SecureNode::" + name in frame.replace(">", "") for frame in stack
                   for name in ("prepare_send", "prepare_receive", "commit_receive")):
                owner[stack[0]] += weight
            if "memmove" in stack[0] or "memcpy" in stack[0]:
                caller = next((name for name in stack[1:] if name.lstrip("<").startswith(
                    ("meshway_dataplane::", "meshway_circuit_perf::", "meshway_static_authority::"))), "unresolved caller")
                copies[caller] += weight
            group = category(stack)
            groups[group] += weight
            role = ("send" if any("SecureNode::send" in s.replace(">", "") for s in stack) else
                    "receive" if any("SecureNode::receive" in s.replace(">", "") for s in stack) else "harness")
            role_groups.setdefault(role, Counter())[group] += weight
    total = sum(own.values())
    if not total:
        raise ValueError("no usable main-thread CPU samples")
    scale = lambda counts: {name: dict(percent=value / total * 100, cpuUsPerPacket=value / total * budget_us)
                            for name, value in counts.items()}
    return dict(samples=samples_seen, unresolvedLeafSamples=unresolved, budgetUs=budget_us,
                weighting="threadCPUDelta where available, otherwise sample count; first 200ms/last 100ms trimmed",
                groups=scale(groups), roles={role: scale(counts) for role, counts in role_groups.items()},
                copyCallers=scale(dict(copies.most_common())),
                functions=[dict(function=name, **values) for name, values in scale(dict(own.most_common())).items()],
                top15=[dict(function=name, **values) for name, values in scale(dict(own.most_common(15))).items()],
                ownerTop15=[dict(function=name, **values) for name, values in scale(dict(owner.most_common(15))).items()])


def summary(report):
    lines = ["# L0 CPU attribution and measured primitive deltas", "",
             "Native attribution = sampled CPU weight × uninstrumented harness CPU/packet; statistical estimate, not exact invocation timing.",
             "Logging scopes are observer-affected; inclusive rows overlap, self estimates are not a partition of uninstrumented CPU.",
             "Savings are primitive substitution arithmetic, not measured optimized L0 or I/O throughput.", "",
             "Primitive windows exclude Frame validation, policy/replay and tag comparison. HMAC fields and contiguous BLAKE3 transcript",
             "are prepared outside timing; candidate AEAD wire AAD updates its sequence inside timing. Production layout changes are unmeasured.", "",
             "Scope ownership: crypto.encrypt/decrypt → pipeline::seal/open → noise::Transport;",
             "crypto.append_mac/verify_hop_mac → session::MacKey → authenticate (includes Frame::validate and the selected HMAC backend/key handling);",
             "codec.encode/decode → Frame::encode_owned/decode_owned; flow.admit → Engine::admit_inner;",
             "secure.send → prepare_send_packet; secure.prepare_receive/commit_receive → owner prepare/commit;",
             "packet.parse → Packet::parse; control.guard → ExecutionSnapshot::reading/ensure_current.", ""]
    primitives = {env: {(row["operation"], row["bytes"]): row["cpuNsPerOp"]["median"] / 1000
                        for row in data["aggregates"]} for env, data in report["primitives"].items()}
    operations = sorted({op for rows in primitives.values() for op, _ in rows})
    lines += ["## Primitive CPU µs/op (medians)", "",
              "| Operation | Host 52B | Host 1280B | Linux 52B | Linux 1280B |", "| --- | ---: | ---: | ---: | ---: |"]
    for op in operations:
        values = [primitives.get(env, {}).get((op, size)) for env in ("host", "linux") for size in (52, 1280)]
        lines.append("| " + op + " | " + " | ".join("unmeasured" if v is None else f"{v:.4f}" for v in values) + " |")
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
                          f"CPU during native recording: {group['samplerCpuUsPerPacket']:.3f} µs/packet (separate longer process).",
                          "", "| Category | CPU % | CPU µs/packet |", "| --- | ---: | ---: |"]
                for name, values in sorted(native["groups"].items()):
                    lines.append(f"| {name} | {values['percent']:.2f} | {values['cpuUsPerPacket']:.3f} |")
                lines += ["", "### Native top 15 self-time", "", "| Function | CPU % | CPU µs/packet |", "| --- | ---: | ---: |"]
                for row in native["top15"]:
                    function = row["function"]
                    if "chacha20::ChaChaCore" in function and "process_with_backend" in function:
                        function = "chacha20::ChaChaCore::process_with_backend<" + ("ApplyBlocksCtx" if "ApplyBlocksCtx" in function else "WriteBlockCtx") + ">"
                    lines.append(f"| `{function.replace('|', '&#124;')}` | {row['percent']:.2f} | {row['cpuUsPerPacket']:.3f} |")
                lines += ["", "### memcpy/memmove callers", "", "These include Rust value moves, not only IP payload copies; byte counts are unmeasured.", "",
                          "| First application caller | CPU % | CPU µs/packet |", "| --- | ---: | ---: |"]
                for name, row in list(native["copyCallers"].items())[:8]:
                    lines.append(f"| `{name.replace('|', '&#124;')}` | {row['percent']:.2f} | {row['cpuUsPerPacket']:.3f} |")
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
                  "Same-repeat deltas describe serial batch variation, not confidence bounds or simultaneous paired execution.", "",
                  "| IP bytes | Nodes | Route | AEAD | Origin saved µs | Endpoint saved µs | Each relay saved µs | Path saved µs | Same-repeat path p10–p90 µs |",
                  "| ---: | ---: | --- | --- | ---: | ---: | ---: | ---: | --- |"]
        for row in rows:
            lines.append(f"| {row['bytes']} | {row['nodes']} | {row['route']} | {row['aead']} | "
                         f"{row['originSavedUs']:.3f} | {row['endpointSavedUs']:.3f} | {row['relaySavedUs']:.3f} | {row['pathSavedUs']:.3f} | "
                         f"{row['pairedPathSavedUs']['p10']:.3f}–{row['pairedPathSavedUs']['p90']:.3f} |")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crypto", type=Path, nargs="+", required=True)
    parser.add_argument("--host-l0", type=Path, required=True)
    parser.add_argument("--linux-l0", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = {"schemaVersion": 1,
              "inputs": {k: [str(p) for p in v] if isinstance(v, list) else str(v) for k, v in vars(args).items()},
              "routes": {}, "l0": {}, "primitives": {}}
    for path in args.crypto:
        crypto = json.loads(path.read_text())
        if crypto.get("sourceUnchangedAfterRun") is not True:
            raise ValueError("primitive sources changed or run has no final source check")
        for env in crypto["environments"]:
            if env["status"] == "ok":
                if env["name"] in report["routes"]:
                    raise ValueError("multiple successful primitive runs for same environment; choose explicitly")
                report["routes"][env["name"]] = routes(env, 2) + routes(env, 3)
                report["primitives"][env["name"]] = dict(source=str(path), binarySha256=env["binarySha256"], aggregates=env["aggregates"])
    for name, path in (("host", args.host_l0), ("linux", args.linux_l0)):
        l0 = json.loads(path.read_text())
        if l0.get("passed") is not True:
            raise ValueError("L0 run did not complete successfully")
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
                recorded = json.loads((path.parent / f"n{nodes}.samply.stdout").read_text())
                item["samplerCpuUsPerPacket"] = recorded["harnessCpuNs"] / recorded["measured"]["deliveredPackets"] / 1000
            item["instrumented"] = next(g for g in l0["aggregates"] if g["nodes"] == nodes and g["profileRate"])
            groups.append(item)
        report["l0"][name] = groups
    circuit.save(args.output, report)
    args.output.with_suffix(".md").write_text(summary(report))


if __name__ == "__main__":
    main()
