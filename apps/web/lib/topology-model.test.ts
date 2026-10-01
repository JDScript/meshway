import assert from "node:assert/strict"
import { test } from "node:test"

import type {
  ConnectionNeighborObservation,
  ConnectionRuntimeObservation,
  ConnectionRuntimesSnapshot,
  ConnectionsSnapshot,
  NodeSummary,
} from "@meshway/control-client"

import {
  zConnectionRuntimeObservation,
  zConnectionsSnapshot,
  zNodeSummary,
} from "../../../packages/control-client/src/generated/zod.gen.ts"
import {
  claimRtt,
  isLive,
  liveBetween,
  observeTopology,
  summarizeNode,
  type Adjacency,
  type Channel,
  type TopologyObservation,
} from "./topology-model.ts"

// Mirrors the demo: nyc dials lax over native UDP, sha dials lax over VLESS;
// the host node has no adjacency at all.
const NYC = "n".repeat(64)
const LAX = "l".repeat(64)
const SHA = "s".repeat(64)
const HOST = "h".repeat(64)
const at = Date.parse("2026-10-01T08:00:00Z")

function node(id: string, name: string, patch: Partial<NodeSummary> = {}): NodeSummary {
  return zNodeSummary.parse({
    id,
    name,
    acknowledged_revision: 9,
    configuration_revision: 9,
    agent_version: "0.0.1",
    country: null,
    last_seen: new Date(at).toISOString(),
    mesh_ipv4: "100.64.0.1",
    mesh_ipv6: "fd00::1",
    registered_at: new Date(at).toISOString(),
    revoked_at: null,
    ...patch,
  })
}

const inventory = [node(NYC, "demo-nyc"), node(LAX, "demo-lax"), node(SHA, "demo-sha"), node(HOST, "nyc")]

function plan(
  extraCandidates: object[] = [],
  revision = 9
): ConnectionsSnapshot {
  return zConnectionsSnapshot.parse({
    revision,
    valid: true,
    diagnostics: [],
    nodes: inventory.map((entry) => ({
      node: entry,
      public_key: null,
      capabilities: [],
      acknowledged_at: null,
      acknowledged_digest: null,
      acknowledged_revision: revision,
    })),
    configuration: {
      listeners: [
        { id: "lax-udp", node_id: LAX, transport: "native_udp", bind: "0.0.0.0:41000", advertised_endpoints: [{ host: "lax.mesh.test", port: 41000 }], credential_ref: null },
        { id: "lax-vless", node_id: LAX, transport: "vless", bind: "0.0.0.0:41001", advertised_endpoints: [{ host: "lax.mesh.test", port: 41001 }], credential_ref: "srv" },
      ],
      candidates: [
        { id: "nyc-to-lax", dialer_node_id: NYC, listener_id: "lax-udp", endpoint: { host: "lax.mesh.test", port: 41000 }, priority: 0, credential_ref: null },
        { id: "sha-to-lax", dialer_node_id: SHA, listener_id: "lax-vless", endpoint: { host: "lax.mesh.test", port: 41001 }, priority: 0, credential_ref: "cli" },
        ...extraCandidates,
      ],
    },
  })
}

function peer(patch: Partial<ConnectionNeighborObservation>): ConnectionNeighborObservation {
  return {
    peer_node_id: LAX,
    transport: "native_udp",
    initiator_node_id: NYC,
    listener_id: "lax-udp",
    candidate_id: null,
    state: "authenticated",
    reason: "none",
    probe_fresh: true,
    probe_rtt_ms: 0,
    ...patch,
  }
}

function report(nodeId: string, peers: ConnectionNeighborObservation[], patch: Partial<ConnectionRuntimeObservation> = {}) {
  return zConnectionRuntimeObservation.parse({
    node_id: nodeId,
    epoch: 1,
    sequence: 10,
    revision: 9,
    digest: "a".repeat(64),
    state: "applied",
    reason: "none",
    listeners: [],
    peers,
    reported_at: new Date(at).toISOString(),
    stale_after_seconds: 60,
    current: true,
    stale: false,
    ...patch,
  })
}

const nycSide = () => report(NYC, [peer({ candidate_id: "nyc-to-lax", probe_rtt_ms: 0 })])
const laxSide = () =>
  report(LAX, [
    peer({ peer_node_id: NYC, initiator_node_id: NYC, probe_rtt_ms: 3 }),
    peer({ peer_node_id: SHA, initiator_node_id: SHA, transport: "vless", listener_id: "lax-vless", probe_rtt_ms: 0 }),
  ])
const shaSide = () =>
  report(SHA, [peer({ initiator_node_id: SHA, transport: "vless", listener_id: "lax-vless", candidate_id: "sha-to-lax", probe_rtt_ms: 2 })])

const demo = (): ConnectionRuntimesSnapshot => ({ nodes: [nycSide(), laxSide(), shaSide()] })

const pair = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)

function adjacencyOf(view: TopologyObservation, a: string, b: string): Adjacency {
  const found = view.adjacencies.find((entry) => entry.key === pair(a, b))
  assert.ok(found, "adjacency exists")
  return found
}

function channelAt(adjacency: Adjacency, index = 0): Channel {
  const found = adjacency.channels[index]
  assert.ok(found, "channel exists")
  return found
}

test("two matching reports merge into one confirmed adjacency per pair, never a full mesh", () => {
  const view = observeTopology(plan(), demo(), inventory, at + 5_000)
  assert.deepEqual(
    view.adjacencies.map((adjacency) => [adjacency.key, adjacency.status]),
    [
      [pair(LAX, NYC), "confirmed"],
      [pair(LAX, SHA), "confirmed"],
    ].sort((a, b) => String(a[0]).localeCompare(String(b[0])))
  )
  assert.equal(view.adjacencies.some((adjacency) => adjacency.key === pair(NYC, SHA)), false)
  assert.deepEqual(summarizeNode(view, HOST), { configured: 0, confirmed: 0, oneSided: 0 })
  assert.equal(view.unmatchedReports, 0)
})

test("both sides' RTTs stay separate and 0 means sub-millisecond", () => {
  const view = observeTopology(plan(), demo(), inventory, at)
  const channel = channelAt(adjacencyOf(view, LAX, NYC))
  assert.equal(channel.dialerNodeId, NYC)
  assert.equal(claimRtt(channel.dialer), "<1 ms")
  assert.equal(claimRtt(channel.acceptor), "3 ms")
})

test("reverse VLESS initiator: sha dials lax, not the other way", () => {
  const view = observeTopology(plan(), demo(), inventory, at)
  const channel = channelAt(adjacencyOf(view, LAX, SHA))
  assert.equal(channel.dialerNodeId, SHA)
  assert.equal(channel.acceptorNodeId, LAX)
  assert.equal(channel.transport, "vless")
  assert.equal(claimRtt(channel.dialer), "2 ms")
})

test("a single side's proof is one-sided and pending, not confirmed", () => {
  const view = observeTopology(plan(), { nodes: [nycSide()] }, inventory, at)
  const adjacency = adjacencyOf(view, LAX, NYC)
  assert.equal(adjacency.status, "one_sided")
  assert.equal(channelAt(adjacency).acceptor.kind, "unusable")
  assert.equal(isLive(adjacency), true)
  assert.equal(adjacencyOf(view, LAX, SHA).status, "unobserved")
})

test("reports age out at the TTL without a new read", () => {
  const view = observeTopology(plan(), demo(), inventory, at + 60_000)
  assert.ok(view.adjacencies.every((adjacency) => adjacency.status === "unobserved"))
  assert.ok(view.adjacencies.every((adjacency) => !isLive(adjacency)))
})

test("a report of an older configuration revision is denied", () => {
  const view = observeTopology(plan([], 10), demo(), inventory, at)
  assert.ok(view.adjacencies.every((adjacency) => adjacency.status === "unobserved"))
  const reason = view.reports.get(NYC)
  assert.ok(reason && !reason.usable && reason.reason === "revision")
})

test("a not-current report is denied even at the same revision", () => {
  const runtimes = { nodes: demo().nodes.map((entry) => ({ ...entry, current: false })) }
  const view = observeTopology(plan(), runtimes, inventory, at)
  assert.ok(view.adjacencies.every((adjacency) => adjacency.status === "unobserved"))
})

test("a candidate ID retargeted to another listener does not inherit the old observation", () => {
  const retargeted = plan()
  retargeted.configuration.candidates = retargeted.configuration.candidates.map((candidate) =>
    candidate.id === "nyc-to-lax" ? { ...candidate, listener_id: "lax-vless" } : candidate
  )
  const view = observeTopology(retargeted, demo(), inventory, at)
  const channel = channelAt(adjacencyOf(view, LAX, NYC))
  assert.equal(channel.dialer.kind, "absent")
  assert.equal(channel.status, "unobserved")
  assert.ok(view.unmatchedReports >= 2)
})

test("a revoked node is ignored on both ends", () => {
  const revoked = inventory.map((entry) => (entry.id === NYC ? { ...entry, revoked_at: new Date(at).toISOString() } : entry))
  const view = observeTopology(plan(), demo(), revoked, at)
  assert.equal(view.adjacencies.some((adjacency) => adjacency.nodeIds.includes(NYC)), false)
  assert.equal(view.reports.get(NYC), undefined)
})

test("withdrawn or errored reports never back a live edge", () => {
  const runtimes = {
    nodes: [nycSide(), report(LAX, laxSide().peers, { state: "withdrawn" }), report(SHA, shaSide().peers, { state: "error", reason: "internal_error" })],
  }
  const view = observeTopology(plan(), runtimes, inventory, at)
  assert.equal(adjacencyOf(view, LAX, NYC).status, "one_sided")
  assert.equal(adjacencyOf(view, LAX, SHA).status, "unobserved")
})

test("an acceptor-only report with a null candidate across standby aliases stays ambiguous and counts once", () => {
  const standby = { id: "nyc-to-lax-b", dialer_node_id: NYC, listener_id: "lax-udp", endpoint: { host: "10.0.0.2", port: 41000 }, priority: 1, credential_ref: null }
  const view = observeTopology(plan([standby]), { nodes: [laxSide()] }, inventory, at)
  const adjacency = adjacencyOf(view, LAX, NYC)
  assert.equal(adjacency.channels.length, 2)
  assert.ok(adjacency.channels.every((channel) => channel.acceptor.kind === "ambiguous"))
  assert.ok(adjacency.channels.every((channel) => channel.status === "unobserved"))
  assert.equal(adjacency.ambiguousProof, true)
  assert.equal(adjacency.status, "one_sided")
  assert.equal(view.adjacencies.filter((entry) => entry.key === pair(LAX, NYC)).length, 1)
})

test("the dialer's exact candidate disambiguates the acceptor's null-candidate report", () => {
  const standby = { id: "nyc-to-lax-b", dialer_node_id: NYC, listener_id: "lax-udp", endpoint: { host: "10.0.0.2", port: 41000 }, priority: 1, credential_ref: null }
  const view = observeTopology(plan([standby]), demo(), inventory, at)
  const adjacency = adjacencyOf(view, LAX, NYC)
  assert.equal(channelAt(adjacency, 0).candidateId, "nyc-to-lax")
  assert.equal(channelAt(adjacency, 0).status, "confirmed")
  assert.equal(channelAt(adjacency, 1).status, "unobserved")
  assert.equal(adjacency.ambiguousProof, false)
})

test("an unauthenticated neighbor is degraded, not live", () => {
  const runtimes = {
    nodes: [report(NYC, [peer({ candidate_id: "nyc-to-lax", state: "unreachable", reason: "transport_failed", probe_fresh: false, probe_rtt_ms: null })])],
  }
  const view = observeTopology(plan(), runtimes, inventory, at)
  const adjacency = adjacencyOf(view, LAX, NYC)
  assert.equal(adjacency.status, "degraded")
  assert.equal(isLive(adjacency), false)
})

test("an authenticated neighbor without a fresh probe is not proof", () => {
  const runtimes = { nodes: [report(NYC, [peer({ candidate_id: "nyc-to-lax", probe_fresh: false })]), laxSide()] }
  const view = observeTopology(plan(), runtimes, inventory, at)
  assert.equal(adjacencyOf(view, LAX, NYC).status, "one_sided")
})

test("a report claiming an unconfigured NYC–SHA channel draws nothing", () => {
  const runtimes = {
    nodes: [
      report(NYC, [peer({ candidate_id: "nyc-to-lax" }), peer({ peer_node_id: SHA, candidate_id: "nyc-to-sha", listener_id: "sha-udp" })]),
      laxSide(),
      shaSide(),
    ],
  }
  const view = observeTopology(plan(), runtimes, inventory, at)
  assert.equal(view.adjacencies.some((adjacency) => adjacency.key === pair(NYC, SHA)), false)
  assert.equal(view.unmatchedReports, 1)
})

test("without any runtime read, configured pairs stay unobserved", () => {
  const view = observeTopology(plan(), null, inventory, at)
  assert.equal(view.adjacencies.length, 2)
  assert.ok(view.adjacencies.every((adjacency) => adjacency.status === "unobserved"))
})

test("a planned NYC→SHA hop has no live adjacency behind it; NYC→LAX→SHA hops do", () => {
  const view = observeTopology(plan(), demo(), inventory, at)
  assert.equal(liveBetween(view, NYC, SHA), false)
  assert.equal(liveBetween(view, NYC, LAX), true)
  assert.equal(liveBetween(view, SHA, LAX), true)
  assert.equal(liveBetween(view, HOST, LAX), false)
})
