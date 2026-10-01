import assert from "node:assert/strict";
import { isIP } from "node:net";
import { isDeepStrictEqual } from "node:util";
import type { AgentDataplaneSnapshot, ConnectionsSnapshot, ConnectionRuntimesSnapshot, MeshAccessSnapshot, PolicySnapshot } from "../../packages/control-client/src/index.ts";
import { array, parse, record } from "./local.ts";
import { TUN_MTU, TUN_NAME, TUN_ROUTE_PROTOCOL } from "./mode.ts";

export type PacketIdentity = { node_id: string; ipv4: string; ipv4_prefix: string };
export type PacketControl = { config: ConnectionsSnapshot; runtime: ConnectionRuntimesSnapshot; policy: PolicySnapshot; access: MeshAccessSnapshot };
const DIGEST = /^[0-9a-f]{64}$/;

export function packetIdentity(snapshot: AgentDataplaneSnapshot): PacketIdentity {
  return { node_id: snapshot.payload.node_id, ipv4: snapshot.payload.configuration.node.mesh_ipv4,
    ipv4_prefix: snapshot.payload.configuration.network.ipv4_prefix };
}

/** Generated node bundle + both ACKs must name the admin APIs' current global revision. */
export function packetSnapshotCurrent(snapshot: AgentDataplaneSnapshot, nodeId: string, control: PacketControl): boolean {
  const { payload } = snapshot;
  const { config, policy, access } = control;
  const receipt = config.nodes.find((entry) => entry.node.id === nodeId);
  return receipt !== undefined && Number.isSafeInteger(payload.revision) && payload.revision > 0 && DIGEST.test(snapshot.digest)
    && config.valid && policy.valid && access.valid && payload.access_valid && payload.configuration.policy_valid && payload.connections.valid
    && payload.revision === config.revision && payload.revision === policy.revision && payload.revision === access.revision
    && payload.configuration.revision === payload.revision && payload.connections.revision === payload.revision
    && payload.node_id === nodeId && payload.configuration.node.id === nodeId && payload.connections.node_id === nodeId
    && receipt.node.revoked_at === null && receipt.node.configuration_revision === payload.revision
    && receipt.node.acknowledged_revision === payload.revision && receipt.acknowledged_revision === payload.revision
    && receipt.acknowledged_digest !== null && DIGEST.test(receipt.acknowledged_digest)
    && receipt.acknowledged_at !== null && receipt.public_key !== null && receipt.public_key === payload.connections.public_key
    && receipt.node.mesh_ipv4 === payload.configuration.node.mesh_ipv4
    && isDeepStrictEqual(policy.network, payload.configuration.network);
}

/** Agent-owned structured stdout proof, not an HTTP DTO or an inferred ACK. */
export function packetProof(logs: string, snapshot: AgentDataplaneSnapshot): boolean {
  const expected = packetIdentity(snapshot);
  let latest: Record<string, unknown> | undefined;
  for (const line of logs.split("\n")) {
    try {
      const entry = parse(line);
      if (entry.event === "managed_dataplane") latest = entry;
    } catch { /* Other bounded daemon output is not readiness evidence. */ }
  }
  return latest !== undefined && latest.node_id === expected.node_id && latest.ipv4 === expected.ipv4
    && Number.isSafeInteger(snapshot.payload.revision) && snapshot.payload.revision > 0 && DIGEST.test(snapshot.digest)
    && latest.revision === snapshot.payload.revision && latest.digest === snapshot.digest
    && latest.ipv4_prefix === expected.ipv4_prefix && latest.tun_name === TUN_NAME && latest.mtu === TUN_MTU
    && latest.ipv6 === false && latest.tun_ready === true && latest.route_sink_retained === true
    && latest.reason === "generation_active" && typeof latest.allowed_paths === "number"
    && Number.isSafeInteger(latest.allowed_paths) && latest.allowed_paths >= 0;
}

/** Real kernel observations, with an owned /32 and protocol-242 route + drop guard. */
export function packetInterface(addresses: unknown, routes: unknown, expected: PacketIdentity): boolean {
  assert(isIP(expected.ipv4) === 4 && /^[0-9.]+\/(?:[1-9]|[12][0-9]|3[0-2])$/.test(expected.ipv4_prefix), "invalid expected mesh IPv4");
  const interfaces = array(addresses).map(record);
  if (interfaces.length !== 1) return false;
  const device = interfaces[0]; assert(device);
  if (device.ifname !== TUN_NAME || device.mtu !== TUN_MTU || !array(device.flags).includes("UP")) return false;
  const ipv4 = array(device.addr_info).map(record).filter((entry) => entry.family === "inet");
  if (ipv4.length !== 1 || ipv4[0]?.local !== expected.ipv4 || ipv4[0]?.prefixlen !== 32) return false;
  const entries = array(routes).map(record);
  // The query includes any more-specific conflicting route within the mesh pool.
  if (entries.length !== 2) return false;
  return entries.some((route) => route.dst === expected.ipv4_prefix && route.dev === TUN_NAME
      && route.prefsrc === expected.ipv4 && route.metric === 42759 && String(route.protocol) === TUN_ROUTE_PROTOCOL
      && route.gateway === undefined && (route.type === undefined || route.type === "unicast" || String(route.type) === "1"))
    && entries.some((route) => route.dst === expected.ipv4_prefix && (route.type === "unreachable" || String(route.type) === "7")
      && route.metric === 42760 && String(route.protocol) === TUN_ROUTE_PROTOCOL && route.dev === undefined && route.gateway === undefined);
}

/** Executed explicitly as the recorded user; never opens or edits Agent state. */
export const PROCESS_PROOF = String.raw`
import json, os, pathlib, stat, subprocess, sys
uid, gid = map(int, sys.argv[1:3])
assert uid > 0 and os.getuid() == uid and os.getgid() == gid
status = dict(line.split(":", 1) for line in pathlib.Path("/proc/1/status").read_text().splitlines())
assert list(map(int, status["Uid"].split())) == [uid] * 4
assert list(map(int, status["Gid"].split())) == [gid] * 4
assert status["Groups"].strip() == ""
assert all(int(status[key], 16) == 0x1000 for key in ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"))
assert status["NoNewPrivs"].strip() == "1"
assert pathlib.Path("/proc/1/comm").read_text().strip() == "meshway-agent"
parent = pathlib.Path("/state").stat()
assert parent.st_uid == uid and stat.S_IMODE(parent.st_mode) == 0o700
assert pathlib.Path("/proc/sys/net/ipv4/ip_forward").read_text().strip() == "0"
assert pathlib.Path("/proc/sys/net/ipv6/conf/all/forwarding").read_text().strip() == "0"
assert pathlib.Path("/proc/sys/net/ipv6/conf/default/disable_ipv6").read_text().strip() == "1"
ipv6 = json.loads(subprocess.check_output(["ip", "-j", "-6", "addr", "show", "dev", "meshway0"], text=True, timeout=3))
assert all(not item.get("addr_info", []) for item in ipv6)
assert any(line.split()[1] == "/" and "ro" in line.split()[3].split(",") for line in pathlib.Path("/proc/mounts").read_text().splitlines())
print(json.dumps({"uid": uid, "gid": gid, "net_admin_only": True, "no_new_privs": True, "rootfs_readonly": True}))
`;
