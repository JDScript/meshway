import assert from "node:assert/strict";
import { array, boolean, record, sockets, text } from "../managed-connections/local.ts";
import type { SocketObservation } from "../managed-connections/local.ts";

export const ROLES = ["NYC", "LAX", "SHA"] as const;
export type Role = typeof ROLES[number];
export type ByRole<T> = Record<Role, T>;
export function integer(value: unknown): number {
  assert(typeof value === "number" && Number.isSafeInteger(value) && value >= 0);
  return value;
}
/** Docker versions differ on the CAP_ prefix; the allowed capability set does not. */
export function hasExactCapabilities(value: unknown, expected: readonly string[]): boolean {
  const actual = (value === null ? [] : array(value)).map(text).map((name) => name.replace(/^CAP_/, "")).sort();
  const allowed = [...expected].sort();
  return actual.length === allowed.length && actual.every((name, index) => name === allowed[index]);
}
export type Route = { destination: string; kind: "tun" | "guard" | "underlay" | "missing"; device: string | null; source: string | null; guard: { prefix: string; type: string } | null };
export function route(value: unknown): Route {
  const item = record(value);
  assert(item.kind === "tun" || item.kind === "guard" || item.kind === "underlay" || item.kind === "missing");
  const guard = item.guard === null ? null : record(item.guard);
  return { destination: text(item.destination), kind: item.kind,
    device: item.device === null ? null : text(item.device), source: item.source === null ? null : text(item.source),
    guard: guard === null ? null : { prefix: text(guard.prefix), type: text(guard.type) } };
}
export type Local = SocketObservation & {
  tun: { name: string; index: number; mtu: number; up: boolean; ipv4: string[]; ipv6: string[] }[];
  routes: Route[]; icmp: { InEchos: number; InEchoReps: number; OutEchoReps: number };
  uid: number; capabilities: number; ping_group_range: number[];
  ipv6_default_disabled: boolean;
  agent_pid: number | null; server_running: boolean; saved_revision: number | null;
  ready_events: number;
  dataplane_proof: { node_id: string; revision: number; digest: string; tun_name: string; ipv4: string; mtu: number; ipv6: boolean;
    tun_ready: boolean; allowed_paths: number; route_sink_retained: boolean; reason: string } | null;
};
export function local(value: Record<string, unknown>): Local {
  const icmp = record(value.icmp);
  const candidate = value.dataplane_proof === null || value.dataplane_proof === undefined ? null : record(value.dataplane_proof);
  const proof = candidate !== null && typeof candidate.revision === "number"
    && Number.isSafeInteger(candidate.revision) && candidate.revision >= 1 && candidate.revision <= 0xffffffff
    && typeof candidate.digest === "string" && /^[0-9a-f]{64}$/.test(candidate.digest) ? candidate : null;
  return { ...sockets(value), tun: array(value.tun).map((entry) => {
    const item = record(entry);
    return { name: text(item.name), index: integer(item.index), mtu: integer(item.mtu), up: boolean(item.up), ipv4: array(item.ipv4).map(text), ipv6: array(item.ipv6).map(text) };
  }), routes: array(value.routes).map(route),
  icmp: { InEchos: integer(icmp.InEchos), InEchoReps: integer(icmp.InEchoReps), OutEchoReps: integer(icmp.OutEchoReps) },
  uid: integer(value.uid), capabilities: integer(value.capabilities), ping_group_range: array(value.ping_group_range).map(integer),
  ipv6_default_disabled: boolean(value.ipv6_default_disabled),
  agent_pid: value.agent_pid === null ? null : integer(value.agent_pid), server_running: boolean(value.server_running),
  saved_revision: value.saved_revision === null ? null : integer(value.saved_revision),
  ready_events: integer(value.ready_events),
  dataplane_proof: proof === null ? null : { node_id: text(proof.node_id), revision: integer(proof.revision), digest: text(proof.digest), tun_name: text(proof.tun_name),
    ipv4: text(proof.ipv4), mtu: integer(proof.mtu), ipv6: boolean(proof.ipv6), tun_ready: boolean(proof.tun_ready),
    allowed_paths: integer(proof.allowed_paths), route_sink_retained: boolean(proof.route_sink_retained), reason: text(proof.reason) } };
}
export type Probe = { ping_exit: number; http_ok: boolean; body_matches: boolean; route: Route };
export function probe(value: Record<string, unknown>): Probe {
  return { ping_exit: integer(value.ping_exit), http_ok: boolean(value.http_ok), body_matches: boolean(value.body_matches), route: route(value.route) };
}
export function noFallback(value: Route): boolean {
  return (value.kind === "tun" && value.device === "meshway0")
    || (value.kind === "guard" && value.guard !== null && ["unreachable", "blackhole", "prohibit"].includes(value.guard.type));
}
export function delivered(value: Probe): boolean {
  return value.ping_exit === 0 && value.http_ok && value.body_matches && value.route.kind === "tun" && noFallback(value.route);
}
export function denied(value: Probe): boolean {
  return value.ping_exit > 0 && !value.http_ok && !value.body_matches && noFallback(value.route);
}
export function tunReady(value: Local, nodeId: string, address: string, revision: number, digest: string): boolean {
  const proof = value.dataplane_proof;
  return value.agent_running && value.saved_revision === revision && value.ready_events > 0
    && proof !== null && proof.node_id === nodeId && proof.ipv4 === address && proof.tun_name === "meshway0"
    && proof.revision === revision && proof.digest === digest
    && proof.mtu === 1000 && !proof.ipv6 && proof.tun_ready && proof.route_sink_retained && proof.reason === "generation_active"
    && value.tun.length === 1 && value.tun[0]?.name === "meshway0" && value.tun[0].up
    && value.tun[0].mtu === 1000 && value.tun[0].ipv4.length === 1 && value.tun[0].ipv4[0] === address && value.tun[0].ipv6.length === 0
    && !value.ipv4_forwarding && !value.ipv6_forwarding && value.ipv6_default_disabled
    && value.routes.every(noFallback);
}
