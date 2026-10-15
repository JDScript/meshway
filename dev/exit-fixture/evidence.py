"""Readback assertions. Observations are evidence, never execution authority."""
import json
import re


def conntrack_flows(text):
    flows = []
    for line in text.splitlines():
        if not line.strip(): continue
        fields = re.findall(r"\b(src|dst|sport|dport)=([^\s]+)", line)
        protocol = re.match(r"(?:ipv4\s+\d+\s+)?(tcp|udp)\s+\d+\s", line)
        if protocol is None or len(fields) != 8 or [key for key, _ in fields] != ["src", "dst", "sport", "dport"] * 2:
            continue  # ICMP/control entries are not TCP/UDP tuple evidence.
        mark = re.search(r"\bmark=(\d+)", line)
        flows.append({"protocol": protocol.group(1), "original": dict(fields[:4]), "reply": dict(fields[4:]),
                      "mark": int(mark.group(1)) if mark else None})
    return flows


def nat_tuple(text, protocol, local, remote, exit_address):
    source, sport = local.rsplit(":", 1)
    destination, dport = remote.rsplit(":", 1)
    found = [flow for flow in conntrack_flows(text) if flow["protocol"] == protocol and flow["original"] ==
             {"src": source, "dst": destination, "sport": sport, "dport": dport}]
    if len(found) != 1:
        raise RuntimeError("missing or ambiguous original conntrack tuple")
    flow = found[0]
    if (flow["reply"]["src"] != destination or flow["reply"]["dst"] != exit_address
            or flow["reply"]["sport"] != dport or not flow["mark"]):
        raise RuntimeError("conntrack does not prove owned masquerade/reverse tuple")
    return flow


def no_owned_exit(snapshot):
    record = snapshot["e"]["nftCounters"]
    if record["exitCode"]:
        raise RuntimeError("nft readback failed")
    nft = json.loads(record["stdout"])
    if any(item.get("table", {}).get("name", "").startswith("meshway_exit") for item in nft["nftables"]):
        raise RuntimeError("owned exit table survived retirement")
    for record in snapshot["e"]["conntrack"].values():
        if record["exitCode"] or any(int(mark) for mark in re.findall(r"\bmark=(\d+)", record["stdout"])):
            raise RuntimeError("owned conntrack survived retirement")


def negative_boundary(record):
    """Expired source authority keeps the owned IPv4 drop gate with no active selector."""
    if record["exitCode"]:
        raise RuntimeError("source nft readback failed")
    items = json.loads(record["stdout"])["nftables"]
    tables = [item["table"]["name"] for item in items if item.get("table", {}).get("name", "").startswith("meshway_egress_")]
    if len(tables) != 1:
        raise RuntimeError("source egress gate missing after lease expiry")
    sets = {item["set"]["name"]: item["set"].get("elem", []) for item in items
            if "set" in item and item["set"]["table"] == tables[0]}
    if not sets.get("managed4") or sets.get("active"):
        raise RuntimeError("source egress gate is not a negative boundary after lease expiry")


def all_stages_passed(manifest):
    return (set(manifest["stages"]) == {str(i) for i in range(1, 17)}
            and all(stage.get("status") == "passed" for stage in manifest["stages"].values())
            and set(manifest.get("processes", {})) == {"controller", "a", "r", "e", "external", "lan", "resolver", "router"}
            and manifest.get("dnsNoSourceFallback", {}).get("passed") is True
            and all(state["ExitCode"] == 0 and not state["Running"] and not state["OOMKilled"] for state in manifest["processes"].values())
            and not manifest.get("cleanupErrors"))
