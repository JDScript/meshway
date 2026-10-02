import assert from "node:assert/strict"
import { test } from "node:test"

import type {
  LatencyHistoryRecord,
  LatencyHistoryResponse,
} from "@meshway/control-client"

import {
  buildHistory,
  mergeRecords,
  msText,
  nextWindow,
  OVERLAP_MS,
  readWindow,
  SPAN_MS,
} from "./latency-model.ts"

const NET = "f".repeat(32)
const A = "a".repeat(64)
const B = "b".repeat(64)
const T0 = Date.parse("2026-10-02T10:00:00Z")

function record(
  id: string,
  at: number,
  patch: Partial<LatencyHistoryRecord["sample"]> = {}
): LatencyHistoryRecord {
  return {
    cursor: id,
    historical: true,
    network_id: NET,
    received_at: new Date(at + 1000).toISOString(),
    source_node_id: A,
    sample: {
      channel_associations: [
        {
          candidate_id: "a-to-b",
          initiator_node_id: A,
          listener_id: "b-udp",
          transport: "native_udp",
        },
      ],
      completed_at: new Date(at + 20).toISOString(),
      digest: "d".repeat(64),
      direction: "source_to_peer",
      failure_reasons: [],
      measured_at: new Date(at).toISOString(),
      measurement_id: id,
      outcome: "success",
      peer_node_id: B,
      revision: 7,
      rtt_us: 20_000,
      runtime_epoch: 1,
      sampler_generation: "g1",
      ...patch,
    },
  }
}

const page = (
  records: LatencyHistoryRecord[],
  next: string | null,
  trimmed: string | null = null
): LatencyHistoryResponse => ({
  capacity_trimmed: trimmed !== null,
  capacity_trimmed_before: trimmed,
  expected_interval_seconds: 30,
  next,
  records,
  retention_from: new Date(T0 - 7 * 86_400_000).toISOString(),
  retention_seconds: 604_800,
})

test("the first window is the span; refreshes overlap the last complete end", () => {
  assert.deepEqual(nextWindow("1h", T0 + 400, null), {
    from: T0 - SPAN_MS["1h"],
    until: T0,
  })
  assert.deepEqual(nextWindow("1h", T0 + 60_000, T0), {
    from: T0 - OVERLAP_MS,
    until: T0 + 60_000,
  })
  // Never earlier than the span on screen.
  assert.equal(
    nextWindow("1h", T0, T0 - 2 * SPAN_MS["1h"]).from,
    T0 - SPAN_MS["1h"]
  )
})

test("a window is read by its own cursor, and only within the page budget", async () => {
  const asked: (string | null)[] = []
  const pages = [
    page([record("1", T0)], "5"),
    page([record("2", T0)], null, new Date(T0).toISOString()),
  ]
  const read = await readWindow(
    async (after) => {
      asked.push(after)
      return pages[asked.length - 1] ?? page([], null)
    },
    { retentionFrom: null, trimmedBefore: null }
  )
  assert.deepEqual(asked, [null, "5"])
  assert.equal(read.complete, true)
  assert.equal(read.records.length, 2)
  assert.equal(read.coverage.trimmedBefore, T0)

  const partial = await readWindow(
    async () => page([record("x", T0)], "9"),
    {
      retentionFrom: null,
      trimmedBefore: null,
    },
    2
  )
  assert.equal(partial.complete, false)
})

test("merging keeps one record per measurement, for the pair and the range", () => {
  const range = { from: T0 - 1000, until: T0 + 10_000 }
  const pair = { networkId: NET, source: A, peer: B }
  const kept = mergeRecords(
    new Map(),
    [record("1", T0), record("old", T0 - 5000)],
    pair,
    range
  )
  const merged = mergeRecords(
    kept,
    [
      record("1", T0),
      record("2", T0 + 30),
      { ...record("3", T0), source_node_id: B },
    ],
    pair,
    range
  )
  assert.deepEqual([...merged.keys()].sort(), ["1", "2"])
})

test("timeouts and failures are marks, not points; gaps and channel changes break the line", () => {
  const window = { from: T0, until: T0 + 600_000 }
  const history = buildHistory(
    [
      record("1", T0 + 30_000),
      record("2", T0 + 60_000),
      record("3", T0 + 90_000, {
        outcome: "timeout",
        rtt_us: null,
        channel_associations: [],
      }),
      record("4", T0 + 120_000),
      record("5", T0 + 400_000),
      record("6", T0 + 430_000, { runtime_epoch: 2 }),
      record("7", T0 + 460_000, {
        outcome: "failed",
        rtt_us: null,
        channel_associations: [],
        failure_reasons: ["channel_unavailable"],
      }),
    ],
    window,
    75_000
  )
  assert.deepEqual(
    history.points.map((point) => point.rtt),
    [20, 20, null, 20, null, 20, null, 20]
  )
  assert.ok(history.points.every((point) => point.rtt !== 0))
  assert.deepEqual(
    history.events.map((event) => [event.outcome, event.reason]),
    [
      ["timeout", null],
      ["failed", "channel_unavailable"],
    ]
  )
  assert.deepEqual(history.changes, [T0 + 430_000])
  assert.equal(history.stats.success, 5)
  assert.equal(history.stats.median, 20)
})

test("no records is no data, not zero", () => {
  const history = buildHistory([], { from: T0, until: T0 + 1 }, 60_000)
  assert.deepEqual(history.points, [])
  assert.equal(history.stats.min, null)
  assert.equal(msText(0.004), "<0.01")
  assert.equal(msText(0.4), "0.40")
})
