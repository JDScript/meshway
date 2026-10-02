import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionNeighborObservation, ConnectionRuntimeObservation, ConnectionsSnapshot, NodeSummary } from "@meshway/control-client"

import { zConnectionRuntimeObservation, zConnectionsSnapshot, zNodeSummary } from "../../../packages/control-client/src/generated/zod.gen.ts"
import {
  edgeFacts,
  nodePlacement,
  observeOverview,
  overviewSummary,
  project,
  relationshipLayout,
  type GeoPlacement,
  type NodePlacement,
} from "./overview-map-model.ts"

const HK = "h".repeat(64)
const SG = "s".repeat(64)
const at = Date.parse("2026-10-01T08:00:00Z")

function node(id: string, name: string, patch: Partial<NodeSummary> = {}): NodeSummary {
  return zNodeSummary.parse({
    id, name, acknowledged_revision: 3, configuration_revision: 3, agent_version: "0.0.1", country: null,
    last_seen: new Date(at).toISOString(), mesh_ipv4: "100.64.0.1", mesh_ipv6: "fd00::1",
    registered_at: new Date(at).toISOString(), revoked_at: null, ...patch,
  })
}
const nodes = [node(HK, "hk-relay", { country: "HK" }), node(SG, "sg-edge", { country: "SG" })]

function plan(candidates = true): ConnectionsSnapshot {
  return zConnectionsSnapshot.parse({
    revision: 3, valid: true, diagnostics: [],
    nodes: nodes.map((entry) => ({ node: entry, public_key: null, capabilities: [], acknowledged_at: null, acknowledged_digest: null, acknowledged_revision: 3 })),
    configuration: {
      listeners: [{ id: "sg-udp", node_id: SG, transport: "native_udp", bind: "0.0.0.0:1", advertised_endpoints: [{ host: "sg.test", port: 1 }], credential_ref: null }],
      candidates: candidates ? [{ id: "hk-to-sg", dialer_node_id: HK, listener_id: "sg-udp", endpoint: { host: "sg.test", port: 1 }, priority: 0, credential_ref: null }] : [],
    },
  })
}

function peer(patch: Partial<ConnectionNeighborObservation>): ConnectionNeighborObservation {
  return { peer_node_id: SG, transport: "native_udp", initiator_node_id: HK, listener_id: "sg-udp", candidate_id: "hk-to-sg", state: "authenticated", reason: "none", probe_fresh: true, probe_rtt_ms: 31, ...patch }
}
function report(nodeId: string, peers: ConnectionNeighborObservation[]): ConnectionRuntimeObservation {
  return zConnectionRuntimeObservation.parse({
    node_id: nodeId, epoch: 1, sequence: 1, revision: 3, digest: "a".repeat(64), state: "applied", reason: "none",
    listeners: [], peers, reported_at: new Date(at).toISOString(), stale_after_seconds: 60, current: true, stale: false,
  })
}

function only<T>(value: T | null | undefined): T {
  assert.ok(value !== null && value !== undefined)
  return value
}

const placements = (list = nodes, sources?: Map<string, GeoPlacement>) =>
  new Map<string, NodePlacement>(list.map((entry) => [entry.id, nodePlacement(entry, sources)]))

test("names, Mesh IPs and the legacy country label never place a node", () => {
  for (const entry of nodes) assert.deepEqual(nodePlacement(entry), { kind: "unplaced" })
  const geo: GeoPlacement = { kind: "geo", lat: 1.3, lon: 103.8, precision: "region", source: "fixture", observedAt: new Date(at).toISOString() }
  assert.deepEqual(nodePlacement(only(nodes[1]), new Map([[SG, geo]])), geo)
  assert.deepEqual(project(0, 0, { north: 82.5, west: -180 }), { x: 180, y: 82.5 })
})

test("the relationship layout is deterministic and does not depend on any location field", () => {
  const a = relationshipLayout(nodes, 400)
  const b = relationshipLayout([...nodes].reverse().map((entry) => ({ ...entry, country: null })), 400)
  assert.deepEqual([...a.entries()], [...b.entries()].sort(([x], [y]) => [...a.keys()].indexOf(x) - [...a.keys()].indexOf(y)))
  assert.deepEqual(relationshipLayout([only(nodes[0])], 400).get(HK), { x: 200, y: 200 })
})

test("connection evidence is unknown without the plan or the runtime reports, never healthy", () => {
  assert.deepEqual(overviewSummary({ nodes, placements: placements(), observation: null, runtimesRead: true }).connections, { kind: "unknown", reason: "plan" })
  const configured = observeOverview(plan(), null, nodes, at)
  assert.deepEqual(overviewSummary({ nodes, placements: placements(), observation: configured, runtimesRead: false }).connections, { kind: "unknown", reason: "runtime" })
  const none = observeOverview(plan(false), { nodes: [] }, nodes, at)
  assert.deepEqual(overviewSummary({ nodes, placements: placements(), observation: none, runtimesRead: true }).connections, { kind: "none" })
  // Read, but nobody reports: configured, not connected.
  const silent = observeOverview(plan(), { nodes: [] }, nodes, at)
  assert.deepEqual(overviewSummary({ nodes, placements: placements(), observation: silent, runtimesRead: true }).connections, { kind: "known", configured: 1, confirmed: 0, oneSided: 0, unverified: 1 })
  assert.equal(edgeFacts(only(only(silent).adjacencies[0])).look, "configured")
  assert.equal(edgeFacts(only(only(silent).adjacencies[0])).live, false)
  assert.equal(overviewSummary({ nodes, placements: placements(), observation: silent, runtimesRead: true }).placed, 0)
})

test("edge facts carry each side's own fresh RTT and nothing from a stale probe", () => {
  const both = observeOverview(plan(), { nodes: [report(HK, [peer({})]), report(SG, [peer({ peer_node_id: HK, candidate_id: null, probe_rtt_ms: 29 })])] }, nodes, at)
  const facts = edgeFacts(only(only(both).adjacencies[0]))
  assert.equal(facts.look, "confirmed")
  assert.deepEqual([facts.channels[0]?.dialer.rtt, facts.channels[0]?.acceptor.rtt], ["31 ms", "29 ms"])

  const staleProbe = observeOverview(plan(), { nodes: [report(HK, [peer({ probe_fresh: false })])] }, nodes, at)
  const stale = edgeFacts(only(only(staleProbe).adjacencies[0]))
  assert.equal(stale.look, "degraded")
  assert.equal(stale.live, false)
  assert.equal(stale.channels[0]?.dialer.rtt, null)
})
