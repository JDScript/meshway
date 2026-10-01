import assert from "node:assert/strict";
import { isIP } from "node:net";
import type { NodeEndpointDiscovery, StunEndpointObservation } from "../../packages/control-client/src/index.ts";
import { array, record, text } from "../managed-connections/local.ts";

export const ROLES = ["one", "two"] as const;
export type Role = typeof ROLES[number];
export function integer(value: unknown): number {
  assert(typeof value === "number" && Number.isSafeInteger(value) && value >= 0);
  return value;
}
export function endpoint(value: string) {
  const match = /^(\d+\.\d+\.\d+\.\d+):(\d+)$/.exec(value);
  assert(match?.[1] && match[2] && isIP(match[1]) === 4);
  const port = Number(match[2]); assert(port > 0 && port <= 65535);
  return { ip: match[1], port };
}
export function freshTimes(observed: string, expires: string, now: number): boolean {
  const start = Date.parse(observed); const end = Date.parse(expires);
  return Number.isFinite(start) && Number.isFinite(end) && start <= now + 1000 && end > now
    && end > start && end - start <= 30_000;
}
export function currentMapped(value: NodeEndpointDiscovery, now = Date.now()) {
  return value.current && !value.stale ? value.sockets.flatMap((socket) => socket.observations.filter((observation) =>
    observation.outcome === "mapped" && observation.mapped_endpoint !== null
    && freshTimes(observation.observed_at, observation.expires_at, now))) : [];
}
export type Peer = { ip: string; port: number; count: number; first_at: number; last_at: number };
export type Ledger = { count: number; peers: Peer[] };
export function ledger(value: Record<string, unknown>): Ledger {
  const result = { count: integer(value.count), peers: array(value.peers).map((item) => {
    const peer = record(item); const ip = text(peer.ip); assert.equal(isIP(ip), 4);
    const first = peer.first_at; const last = peer.last_at;
    assert(typeof first === "number" && Number.isFinite(first) && typeof last === "number" && Number.isFinite(last) && last >= first);
    const port = integer(peer.port); assert(port > 0 && port <= 65535);
    return { ip, port, count: integer(peer.count), first_at: first, last_at: last };
  }) };
  assert(result.peers.length <= 256);
  assert.equal(result.peers.reduce((sum, peer) => sum + peer.count, 0), result.count);
  assert.equal(new Set(result.peers.map((peer) => `${peer.ip}:${peer.port}`)).size, result.peers.length);
  return result;
}
export type Local = { pid: number | null; identity: string; identity_digest: string; key_digest: string | null;
  udp: { family: number; host: string; port: number; inode: string; fd_count: number }[]; ss_udp: string };
export function local(value: Record<string, unknown>): Local {
  assert.equal(integer(value.capabilities), 0); assert.deepEqual(array(value.tun), []);
  const pid = value.agent_pid === null ? null : integer(value.agent_pid);
  assert.equal(integer(value.agent_instances), pid === null ? 0 : 1);
  return { pid, identity: text(value.identity), identity_digest: text(value.identity_digest),
    key_digest: value.key_digest === null ? null : text(value.key_digest), ss_udp: text(value.ss_udp),
    udp: array(value.udp).map((item) => { const entry = record(item);
      return { family: integer(entry.family), host: text(entry.host), port: integer(entry.port),
        inode: text(entry.inode), fd_count: integer(entry.fd_count) };
    }) };
}

/** An HTTP repeat cannot refresh the evidence without a real same-peer STUN event. */
export function noPhantomRenewal(before: StunEndpointObservation, after: StunEndpointObservation, oldPeer: Peer, newPeer: Peer) {
  if (newPeer.count === oldPeer.count) {
    assert.equal(after.observed_at, before.observed_at, "HTTP retransmission changed observation time");
    assert.equal(after.expires_at, before.expires_at, "HTTP retransmission renewed observation TTL");
    assert.deepEqual(after, before, "HTTP retransmission changed mapped evidence without an observer event");
  }
}
export function carrierProof(value: NodeEndpointDiscovery, process: Local, observer: string, ip: string, observed: Ledger, dns = true) {
  assert(value.current && !value.stale && value.epoch > 0 && value.sequence > 0);
  assert(process.pid !== null && process.key_digest !== null);
  const expectedPorts = dns ? [5353] : [];
  const associations = new Set<string>();
  for (const socket of value.sockets) {
    assert(socket.generation > 0 && socket.socket_id > 0);
    const association = `${socket.generation}:${socket.socket_id}`;
    assert(!associations.has(association)); associations.add(association);
    assert.equal(socket.source, "managed_udp_socket_stun");
    const bound = endpoint(socket.bound_address);
    assert([ip, "0.0.0.0"].includes(bound.ip)); expectedPorts.push(bound.port);
    assert(process.udp.some((entry) => entry.family === 4 && entry.port === bound.port && entry.host === bound.ip));
    assert(process.ss_udp.split("\n").some((line) => new RegExp(`:${bound.port}\\s`).test(line)), "carrier absent from ss -lnu");
    if (socket.resource_kind === "listener") assert.equal(bound.port, 41000);
    else assert.notEqual(bound.port, 41000, "outgoing candidate must use its own carrier");
    const mapped = socket.observations.filter((item) => item.outcome === "mapped" && item.mapped_endpoint !== null
      && freshTimes(item.observed_at, item.expires_at, Date.now()));
    assert.equal(mapped.length, 1);
    for (const item of mapped) {
      assert.equal(item.observer, observer); assert(item.mapped_endpoint !== null && item.rtt_ms !== null);
      assert.deepEqual(endpoint(item.mapped_endpoint), { ip, port: bound.port });
      const peer = observed.peers.find((entry) => entry.ip === ip && entry.port === bound.port); assert(peer && peer.count > 0);
      const stamp = Date.parse(item.observed_at) / 1000;
      assert(stamp >= peer.first_at - 2 && stamp <= peer.last_at + 2, "report has no corresponding real observer event");
    }
  }
  assert.equal(new Set(expectedPorts).size, expectedPorts.length, "carrier ports collide");
  assert.deepEqual(process.udp.map((entry) => entry.port).sort((a, b) => a - b), expectedPorts.sort((a, b) => a - b),
    "extra Agent UDP socket: discovery must use carriers, not a temporary probe socket");
  assert(process.udp.every((entry) => entry.family === 4 && entry.fd_count > 0));
}

export function successfulPing(value: Record<string, unknown>, self: string, peer: string) {
  assert.equal(value.exit, 0); const output = record(value.output); assert.equal(output.schema_version, 1);
  assert.equal(record(output.result).status, "ok"); assert.equal(output.stopped_on_failure, false);
  const samples = array(output.samples); assert.equal(samples.length, 1);
  const outcome = record(record(samples[0]).outcome); assert.equal(outcome.status, "ok");
  const proof = record(outcome.result); assert.equal(proof.local_node_id, self); assert.equal(proof.peer_node_id, peer);
  assert.equal(proof.transport, "native_udp"); assert.equal(proof.evidence, "direct_authenticated_neighbor_round_trip");
  integer(proof.rtt_ns); assert(integer(proof.generation) > 0); assert(integer(proof.source_revision) > 0);
  return proof;
}
export function liveStatus(value: Record<string, unknown>, self: string, revision: number) {
  assert.equal(value.exit, 0); const output = record(value.output); assert.equal(output.schema_version, 1);
  const outcome = record(output.outcome); assert.equal(outcome.status, "ok"); const status = record(outcome.result);
  assert.equal(status.local_node_id, self); assert.equal(status.source_revision, revision);
  assert(integer(status.installed_revision) > 0 && integer(status.installed_revision) <= revision);
  assert(integer(status.lease_remaining_ms) > 0); assert.equal(status.evidence, "current_authenticated_channel_registry");
  return status;
}
