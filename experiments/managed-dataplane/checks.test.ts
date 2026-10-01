import assert from "node:assert/strict";
import test from "node:test";
import { delivered, denied, hasExactCapabilities, local, noFallback, tunReady } from "./checks.ts";
import type { Local, Probe, Route } from "./checks.ts";

const route: Route = { destination: "100.96.0.3", kind: "tun", device: "meshway0", source: "100.96.0.1", guard: null };
const passed: Probe = { ping_exit: 0, http_ok: true, body_matches: true, route };
const blocked: Probe = { ping_exit: 1, http_ok: false, body_matches: false, route };
const nodeId = "a".repeat(64);
const digest = "d".repeat(64);
function ready(): Local {
  return { listeners: [], established: [], agent_running: true, controller_running: false,
    agent_udp_ports: [], ipv4_forwarding: false, ipv6_forwarding: false, interfaces: ["meshway0"],
    tun: [{ name: "meshway0", index: 4, mtu: 1000, up: true, ipv4: ["100.96.0.1"], ipv6: [] }],
    routes: [route], icmp: { InEchos: 0, InEchoReps: 0, OutEchoReps: 0 },
    uid: 0, capabilities: 4096, ping_group_range: [0, 2147483647], ipv6_default_disabled: true, agent_pid: 42,
    server_running: true, saved_revision: 17, ready_events: 1,
    dataplane_proof: { node_id: nodeId, revision: 17, digest, tun_name: "meshway0", ipv4: "100.96.0.1", mtu: 1000,
      ipv6: false, tun_ready: true, allowed_paths: 2, route_sink_retained: true, reason: "generation_active" } };
}

test("Docker capability spelling does not broaden the NET_ADMIN-only allowlist", () => {
  assert(hasExactCapabilities(["NET_ADMIN"], ["NET_ADMIN"]));
  assert(hasExactCapabilities(["CAP_NET_ADMIN"], ["NET_ADMIN"]));
  assert(hasExactCapabilities(["ALL"], ["ALL"]));
  assert(hasExactCapabilities(null, []));
  assert(!hasExactCapabilities(null, ["NET_ADMIN"]));
  assert(!hasExactCapabilities(["CAP_NET_ADMIN"], []));
  assert(!hasExactCapabilities(["CAP_NET_ADMIN", "CAP_NET_RAW"], ["NET_ADMIN"]));
  assert(!hasExactCapabilities(["ALL"], ["NET_ADMIN"]));
  assert(!hasExactCapabilities(["CAP_CAP_NET_ADMIN"], ["NET_ADMIN"]));
  assert(!hasExactCapabilities(["NET_ADMIN", "CAP_NET_ADMIN"], ["NET_ADMIN"]));
});

test("delivery requires both kernel ICMP and the exact HTTP canary over TUN", () => {
  assert(delivered(passed));
  assert(!delivered({ ...passed, ping_exit: 1 }));
  assert(!delivered({ ...passed, http_ok: false }));
  assert(!delivered({ ...passed, body_matches: false }));
  assert(!delivered({ ...passed, route: { ...route, kind: "underlay", device: "eth0" } }));
});

test("failed network commands are not sufficient denial evidence", () => {
  assert(denied(blocked));
  assert(!denied({ ...blocked, route: { ...route, kind: "missing", device: null } }));
  assert(!denied({ ...blocked, route: { ...route, kind: "underlay", device: "eth0" } }));
  assert(!denied({ ...blocked, ping_exit: 0 }));
  assert(!denied({ ...blocked, http_ok: true }));
});

test("an explicit unreachable mesh guard is valid after device removal", () => {
  const guard: Route = { ...route, kind: "guard", device: null, source: null, guard: { prefix: "100.96.0.0/16", type: "unreachable" } };
  assert(noFallback(guard));
  assert(denied({ ...blocked, route: guard }));
  assert(!delivered({ ...passed, route: guard }));
  assert(!noFallback({ ...guard, guard: null }));
  assert(!noFallback({ ...guard, guard: { prefix: "100.96.0.0/16", type: "unicast" } }));
});

test("ACK, stale ready log, and an interface alone cannot assert current readiness", () => {
  const value = ready(); assert(tunReady(value, nodeId, "100.96.0.1", 17, digest));
  assert(!tunReady(value, nodeId, "100.96.0.1", 18, digest));
  assert(!tunReady({ ...value, dataplane_proof: null }, nodeId, "100.96.0.1", 17, digest));
  assert(!tunReady({ ...value, tun: [] }, nodeId, "100.96.0.1", 17, digest));
  assert(!tunReady({ ...value, agent_running: false }, nodeId, "100.96.0.1", 17, digest));
  assert(!tunReady({ ...value, ipv6_default_disabled: false }, nodeId, "100.96.0.1", 17, digest));
  assert(value.dataplane_proof);
  assert(!tunReady({ ...value, dataplane_proof: { ...value.dataplane_proof, tun_ready: false, reason: "lease_expired" } }, nodeId, "100.96.0.1", 17, digest));
  assert(!tunReady(value, "b".repeat(64), "100.96.0.1", 17, digest));
  assert(!tunReady(value, nodeId, "100.96.0.2", 17, digest));
});

test("latest saved cache cannot promote an old revision or another snapshot digest", () => {
  const value = ready(); assert(value.dataplane_proof);
  assert.equal(value.saved_revision, 17);
  assert(!tunReady({ ...value, dataplane_proof: { ...value.dataplane_proof, revision: 16 } }, nodeId, "100.96.0.1", 17, digest));
  assert(!tunReady({ ...value, dataplane_proof: { ...value.dataplane_proof, digest: "e".repeat(64) } }, nodeId, "100.96.0.1", 17, digest));
  assert(tunReady(local(value), nodeId, "100.96.0.1", 17, digest));
  assert(!tunReady({ ...value, saved_revision: 16 }, nodeId, "100.96.0.1", 17, digest));
});

test("missing or malformed proof bindings are unready even with the latest saved cache", () => {
  const value = ready(); assert(value.dataplane_proof);
  const { revision, ...withoutRevision } = value.dataplane_proof;
  const { digest: proofDigest, ...withoutDigest } = value.dataplane_proof;
  assert.equal(revision, 17); assert.equal(proofDigest, digest);
  for (const candidate of [withoutRevision, withoutDigest,
    { ...value.dataplane_proof, revision: 0 }, { ...value.dataplane_proof, revision: "17" },
    { ...value.dataplane_proof, digest: "D".repeat(64) }, { ...value.dataplane_proof, digest: "private-text" }]) {
    const observed = local({ ...value, dataplane_proof: candidate });
    assert.equal(observed.saved_revision, 17);
    assert.equal(observed.dataplane_proof, null);
    assert(!tunReady(observed, nodeId, "100.96.0.1", 17, digest));
  }
});
