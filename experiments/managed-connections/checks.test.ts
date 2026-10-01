import assert from "node:assert/strict";
import test from "node:test";
import type { AgentConnectionsSnapshot, ConnectionRuntimeObservation } from "../../packages/control-client/src/index.ts";
import { applied, probed } from "./checks.ts";
import { command } from "./local.ts";

// Pure predicate fixtures only. These objects are never posted to a controller.
const self = "a".repeat(64);
const peer = "b".repeat(64);
const now = Date.parse("2026-09-30T10:00:00.000Z");
const configuration: AgentConnectionsSnapshot = {
  digest: "c".repeat(64),
  payload: {
    schema_version: 1, revision: 7, node_id: self, public_key: "d".repeat(64), valid: true,
    capabilities: [], diagnostics: [], incoming: [], listeners: [], outgoing: [], peers: [],
  },
};
const report: ConnectionRuntimeObservation = {
  node_id: self, epoch: 3, sequence: 9, revision: 7, digest: configuration.digest,
  state: "applied", reason: "none", current: true, stale: false, stale_after_seconds: 60,
  reported_at: new Date(now).toISOString(), listeners: [], peers: [],
};

test("an applied empty plan is valid, without implying a connected peer", () => {
  assert(applied(report, configuration, now));
  assert(!probed(report, peer, self, "vless", "listener", "candidate"));
});

test("stale, non-current, zero epoch, old digest, old revision and missing reports fail", () => {
  for (const change of [
    { stale: true }, { current: false }, { epoch: 0 }, { sequence: 0 },
    { digest: "e".repeat(64) }, { revision: 6 }, { reported_at: null },
    { reported_at: new Date(now - 6_001).toISOString() },
    { reported_at: new Date(now + 2_001).toISOString() },
  ]) assert(!applied({ ...report, ...change }, configuration, now));
  assert(!applied(undefined, configuration, now));
});

test("applied must account for every configured listener", () => {
  const expected: AgentConnectionsSnapshot = {
    ...configuration, payload: { ...configuration.payload, listeners: [{ id: "listener", node_id: self, bind: "0.0.0.0:41001", credential_ref: "local-server", transport: "vless", advertised_endpoints: [] }] },
  };
  assert(!applied(report, expected, now));
  assert(applied({ ...report, listeners: [{ listener_id: "listener", state: "bound", reason: "none" }] }, expected, now));
  assert(!applied({ ...report, listeners: [{ listener_id: "listener", state: "error", reason: "bind_failed" }] }, expected, now));
});

const authenticated: ConnectionRuntimeObservation = {
  ...report,
  peers: [{ peer_node_id: peer, initiator_node_id: self, transport: "vless", listener_id: "listener", candidate_id: "candidate", state: "authenticated", reason: "none", probe_fresh: true, probe_rtt_ms: 0 }],
};

test("measured zero RTT is accepted; unknown RTT and missing control probe are not", () => {
  assert(probed(authenticated, peer, self, "vless", "listener", "candidate"));
  for (const change of [{ probe_rtt_ms: null }, { probe_fresh: false }, { probe_rtt_ms: -1 }]) {
    const original = authenticated.peers[0];
    assert(original);
    assert(!probed({ ...report, peers: [{ ...original, ...change }] }, peer, self, "vless", "listener", "candidate"));
  }
});

test("authenticated observations must match the actual reverse-dial identity and listener", () => {
  assert(!probed(authenticated, peer, peer, "vless", "listener", "candidate"));
  assert(!probed(authenticated, peer, self, "vless", "other-listener", "candidate"));
  assert(!probed(authenticated, peer, self, "native_udp", "listener", "candidate"));
  assert(!probed(authenticated, peer, self, "vless", "listener", "other-candidate"));
  const original = authenticated.peers[0];
  assert(original);
  assert(!probed({ ...report, peers: [{ ...original, candidate_id: null }] }, peer, self, "vless", "listener", "candidate"));
  assert(!probed({ ...report, peers: [{ ...original, reason: "probe_failed" }] }, peer, self, "vless", "listener", "candidate"));
  assert(probed({ ...report, peers: [{ ...original, initiator_node_id: peer, candidate_id: null }] }, peer, peer, "vless", "listener", "candidate"));
});

test("subprocess failures never return captured input, output, or full commands", async () => {
  const sentinel = "unit-test-private-canary";
  await assert.rejects(command(process.execPath, ["-e", "process.stdin.on('data', data => { process.stdout.write(data); process.stderr.write(data); process.exit(7); });"], { input: sentinel }), (error: unknown) => {
    assert(error instanceof Error);
    assert(!error.message.includes(sentinel));
    assert(!error.message.includes("process.stdin"));
    assert(!("stdout" in error) && !("stderr" in error) && !("cmd" in error));
    return true;
  });
});
