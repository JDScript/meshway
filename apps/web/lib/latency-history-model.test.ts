import assert from "node:assert/strict"
import { test } from "node:test"

import type { LatencyHistoryRecord, LatencyHistoryResponse } from "@meshway/control-client"

import { zLatencyHistoryRecord } from "../../../packages/control-client/src/generated/zod.gen.ts"
import { breakAfter, buildHistory, formatMs, readPages, windowRange, type LatencyQuery } from "./latency-history-model.ts"

// Test values only.
const NET = "a".repeat(32)
const SRC = "s".repeat(64)
const PEER = "p".repeat(64)
const t0 = Date.parse("2026-10-02T08:00:00Z")
const query: LatencyQuery = { networkId: NET, sourceNodeId: SRC, peerNodeId: PEER, from: new Date(t0).toISOString(), until: new Date(t0 + 3_600_000).toISOString() }
const hex = (n: number) => n.toString(16).padStart(64, "0")

function record(n: number, offsetS: number, patch: { outcome?: "success" | "timeout" | "failed"; rttUs?: number | null; transport?: "native_udp" | "vless"; epoch?: number; network?: string; source?: string; reason?: "channel_unavailable" } = {}): LatencyHistoryRecord {
  const outcome = patch.outcome ?? "success"
  const at = new Date(t0 + offsetS * 1000).toISOString()
  return zLatencyHistoryRecord.parse({
    cursor: String(n),
    historical: true,
    network_id: patch.network ?? NET,
    source_node_id: patch.source ?? SRC,
    received_at: at,
    sample: {
      measurement_id: hex(n),
      peer_node_id: PEER,
      direction: "source_to_peer",
      runtime_epoch: patch.epoch ?? 1,
      revision: 3,
      digest: "d".repeat(64),
      sampler_generation: "e".repeat(64),
      measured_at: at,
      completed_at: at,
      outcome,
      failure_reasons: outcome === "failed" ? [patch.reason ?? "channel_unavailable"] : [],
      channel_associations: outcome === "success" ? [{ transport: patch.transport ?? "native_udp", initiator_node_id: SRC, listener_id: "l", candidate_id: "c" }] : [],
      rtt_us: outcome === "success" ? (patch.rttUs === undefined ? 1500 : patch.rttUs) : null,
    },
  })
}

test("orders by measured_at, dedupes by measurement id and drops out-of-scope records", () => {
  const history = buildHistory(
    [record(3, 60), record(1, 0), record(2, 30), record(2, 30), record(9, 90, { network: "b".repeat(32) }), record(10, 90, { source: PEER }), record(11, 4000)],
    query,
    breakAfter(30)
  )
  assert.deepEqual(history.successes.map((point) => point.id), [hex(1), hex(2), hex(3)])
  assert.equal(history.ignored, 4)
  assert.equal(history.segments.length, 1)
  assert.equal(history.segments[0]?.runs.length, 1)
})

test("segments split on carrier or epoch changes; failures and timeouts are pair events, not line points", () => {
  const history = buildHistory(
    [record(1, 0), record(2, 30, { outcome: "timeout" }), record(3, 60, { outcome: "failed" }), record(4, 90, { transport: "vless" }), record(5, 120, { epoch: 2, transport: "vless" })],
    query,
    breakAfter(30)
  )
  assert.equal(history.segments.length, 3)
  assert.deepEqual(history.changes.map((change) => change.changed), [["承载"], ["运行代际"]])
  assert.deepEqual(history.events.map((event) => [event.outcome, event.reason]), [["timeout", null], ["failed", "channel_unavailable"]])
  assert.deepEqual([history.stats.success, history.stats.timeout, history.stats.failed], [3, 1, 1])
})

test("a long unrecorded stretch starts a new run; statistics use only successful values", () => {
  const history = buildHistory([record(1, 0, { rttUs: 1000 }), record(2, 30, { rttUs: 3000 }), record(3, 900, { rttUs: 2000 })], query, breakAfter(30))
  assert.equal(history.segments[0]?.runs.length, 2)
  assert.deepEqual([history.stats.minMs, history.stats.medianMs, history.stats.maxMs], [1, 2, 3])
  const empty = buildHistory([record(1, 0, { outcome: "timeout" })], query, breakAfter(30))
  assert.deepEqual([empty.stats.minMs, empty.stats.medianMs, empty.stats.success], [null, null, 0])
})

test("milliseconds keep precision and never show a measured value as zero", () => {
  assert.equal(formatMs(0.001), "<0.01 ms")
  assert.equal(formatMs(0.42), "0.42 ms")
  assert.equal(formatMs(3.25), "3.3 ms")
  assert.equal(formatMs(38.4), "38 ms")
})

test("pages follow the cursor, stop at the budget with a continuation, and honour cancellation", async () => {
  const pages: LatencyHistoryResponse[] = [0, 1, 2].map((index) => ({
    records: [record(index + 1, index * 30)],
    next: index < 2 ? String(index + 1) : null,
    expected_interval_seconds: 30,
    retention_seconds: 604800,
  }))
  const seen: (string | null)[] = []
  const reader = async (after: string | null) => {
    seen.push(after)
    const page = pages[after === null ? 0 : Number(after)]
    assert.ok(page)
    return { status: "loaded" as const, page }
  }
  const all = await readPages(reader)
  assert.equal(all.status, "loaded")
  if (all.status === "loaded") assert.deepEqual([all.records.length, all.next, all.pages], [3, null, 3])
  assert.deepEqual(seen, [null, "1", "2"])
  const truncated = await readPages(reader, { budget: 2 })
  if (truncated.status !== "loaded") assert.fail("expected a page result")
  assert.deepEqual([truncated.records.length, truncated.next], [2, "2"])
  const resumed = await readPages(reader, { after: truncated.next })
  if (resumed.status === "loaded") assert.deepEqual([resumed.records.length, resumed.next], [1, null])
  let calls = 0
  const cancelled = await readPages(async (after) => { calls += 1; return reader(after) }, { cancelled: () => calls >= 1 })
  assert.equal(cancelled.status, "cancelled")
  const failing = await readPages(async () => ({ status: "error", message: "down" }))
  assert.deepEqual(failing, { status: "error", message: "down", records: [] })
})

test("windows are half-open, whole-second and at most 24 hours", () => {
  const range = windowRange("24h", t0 + 1234)
  assert.equal(range.until, new Date(t0 + 1000).toISOString())
  assert.equal(Date.parse(range.until) - Date.parse(range.from), 24 * 3_600_000)
})
