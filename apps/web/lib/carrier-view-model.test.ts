import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionPlan, ConnectionRuntimeObservation, NodeEndpointDiscovery } from "@meshway/control-client"

import { zConnectionRuntimeObservation, zNodeEndpointDiscovery } from "../../../packages/control-client/src/generated/zod.gen.ts"
import { nodeCarrierView, peerProbe, type CarrierView, type UdpFacts } from "./carrier-view-model.ts"

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

function sockets(facts: UdpFacts | undefined) {
  if (facts?.kind !== "sockets") return assert.fail("expected reported sockets")
  return facts.sockets
}

/** Nothing in the view may present itself as current evidence. */
function assertHistory(view: CarrierView) {
  assert.equal(view.discoveryCurrent, false)
  for (const facts of [...view.listeners.map((row) => row.udp), ...view.outgoing.map((row) => row.udp)]) {
    if (facts.kind !== "sockets") continue
    for (const socket of facts.sockets) {
      assert.equal(socket.current, false)
      assert.ok(socket.observed.every((observed) => observed.state !== "candidate"))
    }
  }
  for (const orphan of view.orphans) {
    assert.equal(orphan.facts.current, false)
    assert.ok(orphan.facts.observed.every((observed) => observed.state !== "candidate"))
  }
}

test("a fenced report is current evidence; its socket stays current across a STUN timeout", () => {
  const report = discovery()
  const listenerSocket = report.sockets[0]
  assert.ok(listenerSocket)
  const timedOut = { ...report, sockets: [{ ...listenerSocket, observations: listenerSocket.observations.map((o) => ({ ...o, outcome: "timeout" as const, mapped_endpoint: null, rtt_ms: null })) }] }
  const view = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: timedOut, now })
  assert.equal(view.discoveryCurrent, true)
  const [socket] = sockets(view.listeners[0]?.udp)
  assert.equal(socket?.current, true)
  assert.equal(socket?.observed[0]?.state, "failed")
})

test("same-ID resources after a plan revision change only see the old report as history", () => {
  // Interleaved reads: plan and runtime already at r8, discovery still the server-current r7 response.
  const view = nodeCarrierView({ plan, revision: 8, nodeId: "lax", runtime: runtime({ revision: 8 }), discovery: discovery(), now })
  assertHistory(view)
  assert.equal(sockets(view.listeners[0]?.udp)[0]?.observed[0]?.state, "historical")
  assert.equal(view.listeners[0]?.runtime?.state, "bound")
})

test("discovery from another runtime generation or digest never mixes with the fresh runtime", () => {
  assertHistory(nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime({ epoch: 2 }), discovery: discovery(), now }))
  assertHistory(nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime({ digest: "b".repeat(64) }), discovery: discovery(), now }))
  // Without a usable runtime report there is nothing to fence against.
  assertHistory(nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: undefined, discovery: discovery(), now }))
  assertHistory(nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: discovery({ current: false }), now }))
  assertHistory(nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: discovery({ stale: true }), now }))
})

test("another node's reports never appear on this node", () => {
  const view = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime({ node_id: "nyc" }), discovery: discovery({ node_id: "nyc" }), now })
  assert.equal(view.listeners[0]?.udp.kind, "none")
  assert.equal(view.listeners[0]?.runtime, undefined)
  assert.deepEqual(view.orphans, [])
  assert.equal(view.discoveryCurrent, false)
})

test("orphans and resources moved off UDP are history even in a current report", () => {
  const view = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: discovery(), now })
  assert.equal(view.discoveryCurrent, true)
  const removed = view.orphans.find((orphan) => orphan.resourceId === "removed-udp")
  assert.equal(removed?.facts.current, false)

  const tcpPlan: ConnectionPlan = { ...plan, listeners: plan.listeners.map((listener) => listener.id === "lax-udp" ? { ...listener, transport: "native_tcp" } : listener) }
  const moved = nodeCarrierView({ plan: tcpPlan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: discovery(), now })
  assert.equal(moved.listeners[0]?.udp.kind, "not_applicable")
  const old = moved.orphans.find((orphan) => orphan.resourceId === "lax-udp")
  assert.equal(old?.facts.current, false)
  assert.equal(old?.facts.observed[0]?.state, "historical")
})

test("leases expire on the shared clock and never come back", () => {
  const live = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: discovery(), now })
  assert.equal(sockets(live.listeners[0]?.udp)[0]?.observed[0]?.state, "candidate")
  const expired = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: discovery(), now: now + 20_000 })
  assert.equal(sockets(expired.listeners[0]?.udp)[0]?.observed[0]?.state, "expired")
  assert.equal(sockets(expired.listeners[0]?.udp)[0]?.current, false)
  assert.equal(sockets(expired.outgoing[0]?.udp)[0]?.current, false)
  // The runtime report ages past its TTL: the socket binding is history too.
  const aged = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: discovery(), now: now + 60_000 })
  assertHistory(aged)
})

test("a socket binding lives only as long as its own measurements", () => {
  // A newer runtime report of the same generation does not extend old socket observations.
  const later = now + 25_000
  const refreshed = runtime({ reported_at: new Date(later - 1_000).toISOString() })
  const view = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: refreshed, discovery: discovery(), now: later })
  // The report itself still passes the generation fence; its sockets do not.
  assert.equal(view.discoveryCurrent, true)
  for (const facts of [view.listeners[0]?.udp, view.outgoing[0]?.udp]) {
    for (const socket of sockets(facts)) {
      assert.equal(socket.current, false)
      assert.ok(socket.observed.every((observed) => observed.state !== "candidate"))
    }
  }

  // A fresh timeout still proves the socket existed: its binding is current, the mapping is not.
  const report = discovery()
  const fresh = report.sockets.map((socket) => ({
    ...socket,
    observations: socket.observations.map((observation) => ({ ...observation, outcome: "timeout" as const, mapped_endpoint: null, rtt_ms: null, observed_at: at(-1_000), expires_at: at(20_000) })),
  }))
  const timedOut = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: { ...report, sockets: fresh }, now })
  assert.equal(sockets(timedOut.outgoing[0]?.udp)[0]?.current, true)
  assert.equal(sockets(timedOut.outgoing[0]?.udp)[0]?.observed[0]?.state, "failed")

  // A measurement dated in the future is not evidence for now.
  const future = report.sockets.map((socket) => ({
    ...socket,
    observations: socket.observations.map((observation) => ({ ...observation, observed_at: at(5_000), expires_at: at(20_000) })),
  }))
  const ahead = nodeCarrierView({ plan, revision: 7, nodeId: "lax", runtime: runtime(), discovery: { ...report, sockets: future }, now })
  assert.equal(sockets(ahead.listeners[0]?.udp)[0]?.current, false)
  assert.notEqual(sockets(ahead.listeners[0]?.udp)[0]?.observed[0]?.state, "candidate")
})

test("an authenticated channel without a fresh probe shows no current RTT", () => {
  const peer = runtime().peers[0]
  assert.ok(peer)
  assert.deepEqual(peerProbe(peer), { fresh: true, rtt: "9 ms" })
  assert.deepEqual(peerProbe({ ...peer, probe_fresh: false }), { fresh: false, rtt: null })
  assert.deepEqual(peerProbe({ ...peer, probe_rtt_ms: 0 }), { fresh: true, rtt: "<1 ms" })
  assert.deepEqual(peerProbe({ ...peer, state: "waiting", probe_fresh: true }), { fresh: false, rtt: null })
})
