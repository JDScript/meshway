import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionPlan, ConnectionRuntimeObservation, NodeEndpointDiscovery } from "@meshway/control-client"

import { zConnectionRuntimeObservation, zNodeEndpointDiscovery } from "../../../packages/control-client/src/generated/zod.gen.ts"
import { nodeCarrierView } from "./carrier-view-model.ts"

const now = Date.parse("2026-10-01T08:00:00Z")
const at = (offset: number) => new Date(now + offset).toISOString()

const plan: ConnectionPlan = {
  listeners: [
    { id: "lax-udp", node_id: "lax", transport: "native_udp", bind: "0.0.0.0:41641", advertised_endpoints: [{ host: "203.0.113.5", port: 41641 }], credential_ref: null },
    { id: "lax-vless", node_id: "lax", transport: "vless", bind: "0.0.0.0:443", advertised_endpoints: [{ host: "2001:db8::5", port: 443 }], credential_ref: "p" },
    { id: "nyc-udp", node_id: "nyc", transport: "native_udp", bind: "0.0.0.0:41641", advertised_endpoints: [], credential_ref: null },
  ],
  candidates: [
    { id: "sha-to-lax", dialer_node_id: "sha", listener_id: "lax-udp", endpoint: { host: "203.0.113.5", port: 41641 }, priority: 1, credential_ref: null },
    { id: "lax-to-nyc", dialer_node_id: "lax", listener_id: "nyc-udp", endpoint: { host: "198.51.100.9", port: 41641 }, priority: 2, credential_ref: null },
  ],
}

function runtime(patch: Partial<ConnectionRuntimeObservation> = {}): ConnectionRuntimeObservation {
  return zConnectionRuntimeObservation.parse({
    node_id: "lax", epoch: 1, sequence: 1, revision: 7, digest: "a".repeat(64), state: "applied", reason: "none",
    listeners: [{ listener_id: "lax-udp", state: "bound", reason: "none" }, { listener_id: "lax-vless", state: "error", reason: "bind_failed" }],
    peers: [
      { peer_node_id: "sha", transport: "native_udp", initiator_node_id: "sha", listener_id: "lax-udp", candidate_id: null, state: "authenticated", reason: "none", probe_fresh: true, probe_rtt_ms: 9 },
      { peer_node_id: "nyc", transport: "native_udp", initiator_node_id: "lax", listener_id: "nyc-udp", candidate_id: "lax-to-nyc", state: "unreachable", reason: "probe_failed", probe_fresh: false, probe_rtt_ms: null },
    ],
    reported_at: at(-5_000), stale_after_seconds: 60, current: true, stale: false, ...patch,
  })
}

function discovery(patch: Partial<NodeEndpointDiscovery> = {}): NodeEndpointDiscovery {
  const socket = (kind: "listener" | "outgoing_candidate", id: string, bound: string, mapped: string | null, expires = 20_000) => ({
    resource_kind: kind, resource_id: id, generation: 2, socket_id: kind === "listener" ? 1 : 2, bound_address: bound, source: "managed_udp_socket_stun",
    observations: [{ observer: "192.0.2.1:3478", outcome: mapped ? "mapped" : "timeout", mapped_endpoint: mapped, rtt_ms: mapped ? 4 : null, observed_at: at(-1_000), expires_at: at(expires) }],
  })
  return zNodeEndpointDiscovery.parse({
    node_id: "lax", epoch: 1, sequence: 4, revision: 7, digest: "a".repeat(64), state: "observed", reported_at: at(-1_000), current: true, stale: false,
    sockets: [
      socket("listener", "lax-udp", "0.0.0.0:41641", "198.51.100.1:41641"),
      socket("outgoing_candidate", "lax-to-nyc", "0.0.0.0:53000", "198.51.100.1:53000", -500),
      socket("listener", "removed-udp", "0.0.0.0:40000", null),
    ],
    ...patch,
  })
}

test("listener facts keep configuration, agent binding, sources and peer checks apart", () => {
  const view = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: discovery(), now })
  const [udp, vless] = view.listeners
  assert.ok(udp && vless)
  assert.equal(udp.bind, "0.0.0.0:41641")
  assert.deepEqual(udp.advertised, ["203.0.113.5:41641"])
  assert.equal(udp.runtime?.state, "bound")
  assert.equal(udp.udp.kind, "sockets")
  if (udp.udp.kind === "sockets") {
    assert.equal(udp.udp.sockets[0]?.observed[0]?.state, "candidate")
    assert.equal(udp.udp.sockets[0]?.observed[0]?.address, "198.51.100.1:41641")
  }
  assert.equal(udp.incoming[0]?.peerNodeId, "sha")
  assert.equal(udp.incoming[0]?.reports[0]?.state, "authenticated")
  // A UDP mapping never describes a VLESS (TCP) listener.
  assert.equal(vless.udp.kind, "not_applicable")
  assert.deepEqual(vless.advertised, ["[2001:db8::5]:443"])
  assert.equal(vless.runtime?.reason, "bind_failed")
})

test("outgoing sockets keep their own observations and are never presented as listeners", () => {
  const view = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: discovery(), now })
  const [dial] = view.outgoing
  assert.ok(dial)
  assert.equal(dial.targetNodeId, "nyc")
  assert.equal(dial.udp.kind, "sockets")
  if (dial.udp.kind === "sockets") {
    assert.equal(dial.udp.sockets[0]?.boundAddress, "0.0.0.0:53000")
    assert.equal(dial.udp.sockets[0]?.observed[0]?.state, "expired")
  }
  assert.equal(dial.check.reports[0]?.state, "unreachable")
  assert.ok(view.listeners.every((listener) => listener.id !== "lax-to-nyc"))
  assert.deepEqual(view.orphans.map((orphan) => [orphan.resourceId, orphan.facts.observed[0]?.state]), [["removed-udp", "failed"]])
})

test("a node without listeners still shows the addresses of its dial sockets", () => {
  const dialOnly: ConnectionPlan = {
    listeners: plan.listeners.filter((listener) => listener.node_id === "nyc"),
    candidates: plan.candidates.filter((candidate) => candidate.dialer_node_id === "lax"),
  }
  const view = nodeCarrierView({ plan: dialOnly, revision: 7, nodeId: "lax", runtime: undefined, discovery: discovery(), now })
  assert.equal(view.listeners.length, 0)
  assert.equal(view.outgoing[0]?.udp.kind, "sockets")
})

test("runtime of another revision and non-current discovery never annotate the plan as live", () => {
  const view = nodeCarrierView({ plan, revision: 8, nodeId: "lax", runtime: runtime(), discovery: discovery({ current: false }), now })
  const [udp] = view.listeners
  assert.equal(udp?.runtime, undefined)
  assert.deepEqual(udp?.incoming[0]?.reports, [])
  if (udp?.udp.kind === "sockets") assert.equal(udp.udp.sockets[0]?.observed[0]?.state, "historical")
  else assert.fail("expected the reported socket")
})
