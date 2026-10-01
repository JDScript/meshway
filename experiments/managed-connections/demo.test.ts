import assert from "node:assert/strict";
import test from "node:test";
import type { ConnectionPlan, ConnectionRuntimesSnapshot, ConnectionsSnapshot } from "../../packages/control-client/src/index.ts";
import { assertDemoOwnership, demoEpochFloor, demoHealthy, demoPlan, mergeDemoPlan } from "./demo.ts";

const ids = { nyc: "1".repeat(64), lax: "2".repeat(64), sha: "3".repeat(64) };
const other = "4".repeat(64);

test("demo merge preserves unrelated nodes, values and order across repeated starts", () => {
  const foreign: ConnectionPlan = {
    listeners: [{ id: "local-listener", node_id: other, transport: "native_tcp", bind: "127.0.0.1:49000", advertised_endpoints: [{ host: "private.example", port: 49000 }], credential_ref: null }],
    // IDs are unique per collection in the controller contract; equal unrelated
    // listener/candidate IDs must not make the demo reject a valid global plan.
    candidates: [{ id: "local-listener", dialer_node_id: "5".repeat(64), listener_id: "local-listener", endpoint: { host: "private.example", port: 49000 }, credential_ref: null, priority: 7 }],
  };
  const before = structuredClone(foreign);
  const once = mergeDemoPlan(foreign, ids);
  assert.deepEqual(foreign, before, "input must remain untouched");
  assert.equal(once.listeners[0], foreign.listeners[0]);
  assert.equal(once.candidates[0], foreign.candidates[0]);
  assert.deepEqual(mergeDemoPlan(once, ids), once, "rerun must be a no-op, including order");
  const edited = structuredClone(once);
  const own = edited.listeners.find((entry) => entry.id === "demo-lax-udp");
  assert(own); own.bind = "0.0.0.0:49999";
  assert.deepEqual(mergeDemoPlan(edited, ids), once, "only explicitly owned entries are restored");
});

test("reserved IDs are not ownership: foreign listener, dialer, target and cross-kind collisions reject", () => {
  for (const change of [
    (plan: ConnectionPlan) => { const entry = plan.listeners[0]; assert(entry); entry.node_id = other; },
    (plan: ConnectionPlan) => { const entry = plan.candidates[0]; assert(entry); entry.dialer_node_id = other; },
    (plan: ConnectionPlan) => { const entry = plan.candidates[0]; assert(entry); entry.listener_id = "someone-elses-listener"; },
    (plan: ConnectionPlan) => { const entry = plan.candidates[0]; assert(entry); plan.listeners = []; entry.id = "demo-lax-udp"; },
  ]) {
    const plan = demoPlan(ids); change(plan);
    const before = structuredClone(plan);
    assert.throws(() => mergeDemoPlan(plan, ids), /different owner/);
    assert.deepEqual(plan, before);
  }
  const duplicate = demoPlan(ids);
  const first = duplicate.candidates[0]; assert(first);
  duplicate.candidates.push({ ...first });
  assert.throws(() => mergeDemoPlan(duplicate, ids), /duplicate/);
});

test("partial owned plan is repaired without dropping unrelated entries", () => {
  const full = demoPlan(ids);
  const listener = full.listeners[1]; const candidate = full.candidates[1];
  assert(listener && candidate);
  const partial = { listeners: [listener], candidates: [candidate] };
  const merged = mergeDemoPlan(partial, ids);
  assert.equal(merged.listeners[0]?.id, listener.id);
  assert.equal(merged.candidates[0]?.id, candidate.id);
  assert.deepEqual(new Set(merged.listeners.map((entry) => entry.id)), new Set(full.listeners.map((entry) => entry.id)));
  assert.deepEqual(mergeDemoPlan(merged, ids), merged);
});

test("Docker ownership requires project, run, data directory fingerprint AND exact resource", () => {
  const owner = { run_id: "a".repeat(32), fingerprint: "b".repeat(64) };
  const labels = { "io.meshway.demo.project": "managed-neighbors", "io.meshway.demo.run": owner.run_id,
    "io.meshway.demo.data": owner.fingerprint, "io.meshway.demo.resource": "lax" };
  assert.doesNotThrow(() => assertDemoOwnership(labels, owner, "lax"));
  for (const key of Object.keys(labels)) {
    assert.throws(() => assertDemoOwnership({ ...labels, [key]: "different-owner" }, owner, "lax"), /ownership/);
    assert.throws(() => assertDemoOwnership(Object.fromEntries(Object.entries(labels).filter(([name]) => name !== key)), owner, "lax"), /ownership/);
  }
  assert.throws(() => assertDemoOwnership(labels, owner, "lax-enroll"), /ownership/);
  assert.throws(() => assertDemoOwnership(null, owner, "lax"));
});

function graph(now: number): { config: ConnectionsSnapshot; runtime: ConnectionRuntimesSnapshot } {
  const digest = "f".repeat(64);
  const config: ConnectionsSnapshot = {
    revision: 12, valid: true, diagnostics: [], configuration: demoPlan(ids),
    nodes: Object.entries(ids).map(([role, id]) => ({ public_key: "a".repeat(64), capabilities: [],
      acknowledged_at: new Date(now).toISOString(), acknowledged_digest: digest, acknowledged_revision: 12,
      node: { id, name: `demo-${role}`, revoked_at: null, registered_at: new Date(now).toISOString(), last_seen: new Date(now).toISOString(),
        acknowledged_revision: 12, configuration_revision: 12, agent_version: "test", country: null, mesh_ipv4: "100.64.0.1", mesh_ipv6: "fd00::1" } })),
  };
  const runtime: ConnectionRuntimesSnapshot = { nodes: Object.values(ids).map((id) => ({ node_id: id, epoch: 1, sequence: 2,
    revision: 12, digest, state: "applied", reason: "none", reported_at: new Date(now).toISOString(), stale_after_seconds: 60, current: true, stale: false,
    listeners: id === ids.lax ? ["demo-lax-udp", "demo-lax-vless"].map((listener_id) => ({ listener_id, state: "bound", reason: "none" })) : [],
    peers: [],
  })) };
  for (const [dialer, transport, listener, candidate] of [
    [ids.nyc, "native_udp", "demo-lax-udp", "demo-nyc-to-lax"],
    [ids.sha, "vless", "demo-lax-vless", "demo-sha-to-lax"],
  ] as const) {
    for (const observer of [dialer, ids.lax]) {
      const node = runtime.nodes.find((entry) => entry.node_id === observer); assert(node);
      node.peers.push({ peer_node_id: observer === dialer ? ids.lax : dialer, initiator_node_id: dialer, transport,
        listener_id: listener, candidate_id: observer === dialer ? candidate : null,
        state: "authenticated", reason: "none", probe_fresh: true, probe_rtt_ms: 0 });
    }
  }
  return { config, runtime };
}

test("health demands fresh current applied observations, saved matching digests and both measured probe sides", () => {
  const now = Date.now();
  const healthy = graph(now);
  assert(demoHealthy(healthy.config, healthy.runtime, ids, now));
  for (const change of [
    (value: ReturnType<typeof graph>) => { value.runtime.nodes.pop(); },
    (value: ReturnType<typeof graph>) => { const node = value.runtime.nodes[0]; assert(node); node.current = false; },
    (value: ReturnType<typeof graph>) => { const node = value.runtime.nodes[0]; assert(node); node.reported_at = new Date(now - 9_000).toISOString(); },
    (value: ReturnType<typeof graph>) => { const node = value.config.nodes[0]; assert(node); node.acknowledged_digest = "0".repeat(64); },
    (value: ReturnType<typeof graph>) => { const node = value.runtime.nodes[0]; assert(node); node.state = "applying"; },
    (value: ReturnType<typeof graph>) => { const peer = value.runtime.nodes[0]?.peers[0]; assert(peer); peer.probe_fresh = false; },
    (value: ReturnType<typeof graph>) => { const peer = value.runtime.nodes[0]?.peers[0]; assert(peer); peer.probe_rtt_ms = null; },
    (value: ReturnType<typeof graph>) => { const node = value.runtime.nodes[1]; assert(node); node.peers.pop(); },
    (value: ReturnType<typeof graph>) => { value.config.revision++; },
  ]) {
    const value = graph(now); change(value);
    assert.equal(demoHealthy(value.config, value.runtime, ids, now), false);
  }
});

test("VLESS must be SHA-initiated on both sides, and outgoing candidate identity must match", () => {
  const now = Date.now();
  for (const observer of [ids.lax, ids.sha]) {
    const value = graph(now);
    const peer = value.runtime.nodes.find((node) => node.node_id === observer)?.peers.find((entry) => entry.transport === "vless");
    assert(peer); peer.initiator_node_id = ids.lax;
    assert.equal(demoHealthy(value.config, value.runtime, ids, now), false);
  }
  const value = graph(now);
  const peer = value.runtime.nodes.find((node) => node.node_id === ids.sha)?.peers[0]; assert(peer);
  peer.candidate_id = null;
  assert.equal(demoHealthy(value.config, value.runtime, ids, now), false);
});

test("controlled same-image start requires strictly newer per-node epochs as well as fresh dual probes", () => {
  const now = Date.now(); const value = graph(now);
  for (const [index, node] of value.runtime.nodes.entries()) node.epoch = [4, 7, 12][index] ?? 0;
  const floor = demoEpochFloor(value.runtime, ids);
  assert.deepEqual(floor, { nyc: 4, lax: 7, sha: 12 });
  assert(demoHealthy(value.config, value.runtime, ids, now), "read-only health has no restart threshold");
  assert.equal(demoHealthy(value.config, value.runtime, ids, now, floor), false, "a recent previous process cannot satisfy a controlled start");
  for (const node of value.runtime.nodes) {
    node.epoch++;
    if (node !== value.runtime.nodes.at(-1)) assert.equal(demoHealthy(value.config, value.runtime, ids, now, floor), false);
  }
  assert(demoHealthy(value.config, value.runtime, ids, now, floor));
  const peer = value.runtime.nodes[1]?.peers[1]; assert(peer);
  peer.probe_fresh = false;
  assert.equal(demoHealthy(value.config, value.runtime, ids, now, floor), false, "new epochs cannot replace authenticated probe evidence");
});

test("runtime floors use zero only for missing nodes and retain stale/withdrawn epochs", () => {
  const now = Date.now(); const value = graph(now);
  const node = value.runtime.nodes[1]; assert(node);
  node.epoch = 19; node.stale = true; node.current = false; node.reported_at = null;
  node.state = "withdrawn"; node.listeners = []; node.peers = [];
  value.runtime.nodes = [node];
  assert.deepEqual(demoEpochFloor(value.runtime, ids), { nyc: 0, lax: 19, sha: 0 });
  assert.deepEqual(demoEpochFloor({ nodes: [] }, ids), { nyc: 0, lax: 0, sha: 0 });
  value.runtime.nodes.push({ ...node });
  assert.throws(() => demoEpochFloor(value.runtime, ids), /ambiguous/);
});

test("saved NYC/SHA listeners are preserved and must each be freshly bound; epoch fencing still applies", () => {
  const now = Date.now(); const value = graph(now);
  for (const role of ["nyc", "sha"] as const) {
    const id = `demo-${role}-udp`;
    value.config.configuration.listeners.push({ id, node_id: ids[role], transport: "native_udp", bind: "0.0.0.0:41002",
      advertised_endpoints: [{ host: `${role}.mesh.test`, port: 41002 }], credential_ref: null });
    const node = value.runtime.nodes.find((node) => node.node_id === ids[role]); assert(node);
    node.listeners.push({ listener_id: id, state: "bound", reason: "none" });
  }
  const saved = structuredClone(value.config.configuration);
  const floor = { nyc: 0, lax: 0, sha: 0 };
  assert(demoHealthy(value.config, value.runtime, ids, now, floor));
  assert.equal(demoHealthy(value.config, value.runtime, ids, now, { nyc: 1, lax: 1, sha: 1 }), false);
  assert.deepEqual(value.config.configuration, saved, "health must never repair or drop user-added entries");
  for (const change of [
    (snapshot: ConnectionRuntimesSnapshot) => { snapshot.nodes[0]?.listeners.pop(); },
    (snapshot: ConnectionRuntimesSnapshot) => { const listener = snapshot.nodes[2]?.listeners[0]; assert(listener); listener.state = "error"; listener.reason = "bind_failed"; },
    (snapshot: ConnectionRuntimesSnapshot) => { snapshot.nodes[0]?.listeners.push({ listener_id: "unsaved-listener", state: "bound", reason: "none" }); },
  ]) {
    const runtime = structuredClone(value.runtime); change(runtime);
    assert.equal(demoHealthy(value.config, runtime, ids, now, floor), false);
    assert.deepEqual(value.config.configuration, saved);
  }
});

test("additional saved candidates may report scoped peers but cannot replace required edges or invent claims", () => {
  const now = Date.now(); const value = graph(now);
  value.config.configuration.candidates.push({ id: "extra-sha-udp", dialer_node_id: ids.sha, listener_id: "demo-lax-udp",
    endpoint: { host: "lax.mesh.test", port: 41000 }, priority: 1, credential_ref: null });
  const sha = value.runtime.nodes.find((node) => node.node_id === ids.sha); assert(sha);
  const lax = value.runtime.nodes.find((node) => node.node_id === ids.lax); assert(lax);
  sha.peers.push({ candidate_id: "extra-sha-udp", initiator_node_id: ids.sha, listener_id: "demo-lax-udp", peer_node_id: ids.lax,
    transport: "native_udp", state: "waiting", reason: "none", probe_fresh: false, probe_rtt_ms: null });
  lax.peers.push({ candidate_id: null, initiator_node_id: ids.sha, listener_id: "demo-lax-udp", peer_node_id: ids.sha,
    transport: "native_udp", state: "authenticated", reason: "none", probe_fresh: true, probe_rtt_ms: 1 });
  const saved = structuredClone(value.config.configuration);
  assert(demoHealthy(value.config, value.runtime, ids, now));
  for (const change of [
    (snapshot: ConnectionRuntimesSnapshot) => { const node = snapshot.nodes[2]; assert(node); node.peers.shift(); },
    (snapshot: ConnectionRuntimesSnapshot) => { const peer = snapshot.nodes[2]?.peers.at(-1); assert(peer); peer.candidate_id = null; },
    (snapshot: ConnectionRuntimesSnapshot) => { const peer = snapshot.nodes[2]?.peers.at(-1); assert(peer); peer.candidate_id = "foreign-candidate"; },
    (snapshot: ConnectionRuntimesSnapshot) => { const peer = snapshot.nodes[2]?.peers.at(-1); assert(peer); peer.transport = "native_tcp"; },
    (snapshot: ConnectionRuntimesSnapshot) => { const peer = snapshot.nodes[2]?.peers.at(-1); assert(peer); peer.peer_node_id = ids.nyc; },
    (snapshot: ConnectionRuntimesSnapshot) => { const peer = snapshot.nodes[1]?.peers.at(-1); assert(peer); peer.initiator_node_id = ids.nyc; },
    (snapshot: ConnectionRuntimesSnapshot) => { const node = snapshot.nodes[1]; const peer = node?.peers.at(-1); assert(node && peer); node.peers.push({ ...peer }); },
  ]) {
    const runtime = structuredClone(value.runtime); change(runtime);
    assert.equal(demoHealthy(value.config, runtime, ids, now), false);
  }
  assert.deepEqual(value.config.configuration, saved);
});
