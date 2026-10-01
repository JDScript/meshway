import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionRuntimeObservation } from "@meshway/control-client"

import { zConnectionRuntimeObservation } from "../../../packages/control-client/src/generated/zod.gen.ts"
import {
  candidateNeighbors,
  candidateRowNeighbors,
  formatRtt,
  listenerRowObservation,
  neighborCounts,
  neighborLabel,
  reportMatchesBase,
  runtimeFreshness,
  runtimeSummary,
} from "./connection-runtime-model.ts"

const receivedAt = Date.parse("2026-10-01T08:00:00Z")
const digest = "a".repeat(64)

function observation(patch: Partial<ConnectionRuntimeObservation> = {}): ConnectionRuntimeObservation {
  return zConnectionRuntimeObservation.parse({
    node_id: "lax",
    epoch: 1,
    sequence: 3,
    revision: 7,
    digest,
    state: "applied",
    reason: "none",
    listeners: [{ listener_id: "lax-udp", state: "bound", reason: "none" }],
    peers: [
      {
        peer_node_id: "sha",
        transport: "native_udp",
        initiator_node_id: "sha",
        listener_id: "lax-udp",
        candidate_id: null,
        state: "authenticated",
        reason: "none",
        probe_fresh: true,
        probe_rtt_ms: 42,
      },
      {
        peer_node_id: "nyc",
        transport: "native_udp",
        initiator_node_id: "nyc",
        listener_id: "lax-udp",
        candidate_id: "nyc-to-lax-udp",
        state: "waiting",
        reason: "none",
        probe_fresh: false,
        probe_rtt_ms: null,
      },
    ],
    reported_at: new Date(receivedAt).toISOString(),
    stale_after_seconds: 60,
    current: true,
    stale: false,
    ...patch,
  })
}

test("a missing observation is unreported, never assumed disconnected", () => {
  assert.equal(runtimeFreshness(undefined, receivedAt), "unreported")
})

test("a lease without a report is awaiting, even with server flags", () => {
  const awaiting = observation({
    state: "awaiting_report",
    sequence: 0,
    revision: null,
    digest: null,
    reported_at: null,
    listeners: [],
    peers: [],
    current: false,
    stale: true,
  })
  assert.equal(runtimeFreshness(awaiting, receivedAt), "awaiting")
})

test("freshness honours the current flag, the server stale flag and local ageing", () => {
  assert.equal(runtimeFreshness(observation(), receivedAt + 10_000), "fresh")
  assert.equal(runtimeFreshness(observation({ current: false, stale: true }), receivedAt), "outdated")
  assert.equal(runtimeFreshness(observation({ stale: true }), receivedAt + 1_000), "expired")
  // Fetched while fresh, but the page kept it on screen past the TTL.
  assert.equal(runtimeFreshness(observation(), receivedAt + 60_000), "expired")
  assert.equal(runtimeFreshness(observation({ state: "withdrawn", listeners: [], peers: [] }), receivedAt), "withdrawn")
})

test("applied summarises listener state only; neighbors are counted separately", () => {
  const applied = observation()
  assert.equal(runtimeSummary(applied), "连接配置已应用")
  assert.deepEqual(neighborCounts(applied), { authenticated: 1, waiting: 1, unreachable: 0 })
  assert.equal(runtimeSummary(observation({ state: "error", reason: "bind_failed", listeners: [], peers: [] })), "错误：本机监听失败")
})

const laxUdp = { nodeId: "lax", transport: "native_udp" as const }

test("acceptor-side neighbors match the whole relation, with or without a candidate ID", () => {
  const report = observation()
  const relation = (id: string, dialer: string) => ({
    id,
    dialerNodeId: dialer,
    listenerId: "lax-udp",
    acceptorNodeId: "lax",
    transport: "native_udp" as const,
  })
  assert.deepEqual(candidateNeighbors(report, relation("sha-to-lax-udp", "sha"), "lax").map((p) => p.peer_node_id), ["sha"])
  assert.deepEqual(candidateNeighbors(report, relation("nyc-to-lax-udp", "nyc"), "lax").map((p) => p.peer_node_id), ["nyc"])
  // Same carrier and listener, but a different transport is a different adjacency.
  assert.deepEqual(candidateNeighbors(report, { ...relation("sha-to-lax-udp", "sha"), transport: "native_tcp" }, "lax"), [])
  assert.deepEqual(candidateNeighbors(undefined, relation("x", "sha"), "lax"), [])
})

function dialerReport(patch: Partial<ConnectionRuntimeObservation> = {}) {
  return observation({
    node_id: "sha",
    listeners: [],
    peers: [
      {
        peer_node_id: "lax",
        transport: "native_udp",
        initiator_node_id: "sha",
        listener_id: "lax-udp",
        candidate_id: "sha-out",
        state: "authenticated",
        reason: "none",
        probe_fresh: true,
        probe_rtt_ms: 0,
      },
    ],
    ...patch,
  })
}

test("a retargeted candidate that keeps its ID never inherits the old target's proof", () => {
  const snapshot = { nodes: [dialerReport()] }
  const now = receivedAt + 1_000
  const toLax = { id: "sha-out", dialerNodeId: "sha", listenerId: "lax-udp" }
  assert.equal(candidateRowNeighbors(snapshot, now, 7, toLax, laxUdp, true).length, 1)
  // Edited to dial a TYO listener under the same ID: changed against the base.
  const toTyo = { ...toLax, listenerId: "tyo-udp" }
  const tyoUdp = { nodeId: "tyo", transport: "native_udp" as const }
  assert.deepEqual(candidateRowNeighbors(snapshot, now, 7, toTyo, tyoUdp, false), [])
  // Even if the change flag were wrong, the relation itself does not match.
  assert.deepEqual(candidateRowNeighbors(snapshot, now, 7, toTyo, tyoUdp, true), [])
})

test("row badges require a fresh report of exactly the editor's saved base revision", () => {
  const now = receivedAt + 1_000
  const toLax = { id: "sha-out", dialerNodeId: "sha", listenerId: "lax-udp" }
  // The report is current for r7, but this editor's draft is based on r6.
  assert.deepEqual(candidateRowNeighbors({ nodes: [dialerReport()] }, now, 6, toLax, laxUdp, true), [])
  assert.equal(reportMatchesBase(dialerReport(), 7, now), true)
  assert.equal(reportMatchesBase(dialerReport({ current: false, stale: true }), 7, now), false)

  const lax = { nodes: [observation()] }
  assert.equal(listenerRowObservation(lax, now, 7, { id: "lax-udp", nodeId: "lax" }, true)?.state, "bound")
  // Bind or transport edited in the draft: nothing applied to show yet.
  assert.equal(listenerRowObservation(lax, now, 7, { id: "lax-udp", nodeId: "lax" }, false), undefined)
  assert.equal(listenerRowObservation(lax, now, 6, { id: "lax-udp", nodeId: "lax" }, true), undefined)
})

test("RTT is only shown for authenticated neighbors with a measurement", () => {
  const [authenticated, waiting] = observation().peers
  assert.ok(authenticated && waiting)
  assert.equal(formatRtt(authenticated), "42 ms")
  assert.equal(formatRtt(waiting), "未测")
  assert.equal(formatRtt({ ...authenticated, probe_rtt_ms: null }), "未测")
})

test("a peer failure is shown on that peer without failing the applied configuration", () => {
  const report = observation({
    peers: [
      {
        peer_node_id: "sha",
        transport: "vless",
        initiator_node_id: "lax",
        listener_id: "sha-vless",
        candidate_id: "lax-to-sha-vless",
        state: "unreachable",
        reason: "authentication_failed",
        probe_fresh: false,
        probe_rtt_ms: null,
      },
    ],
  })
  assert.equal(runtimeSummary(report), "连接配置已应用")
  const [peer] = report.peers
  assert.ok(peer)
  assert.equal(neighborLabel(peer), "不可达：节点认证失败")
  assert.equal(formatRtt(peer), "未测")
})

test("a zero-millisecond probe is shown as sub-millisecond, not as missing", () => {
  const [peer] = dialerReport().peers
  assert.ok(peer)
  assert.equal(formatRtt(peer), "<1 ms")
})
