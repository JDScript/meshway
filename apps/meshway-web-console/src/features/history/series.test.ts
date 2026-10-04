import assert from "node:assert/strict"
import { test } from "node:test"
import type { HistoryBucket } from "@meshway/control-client"

import {
  bucketSeries,
  coverageFacts,
  lineRuns,
  unansweredRate,
} from "./series.ts"
import { historyWindow } from "./window.ts"

const A = "aaaaaaaa-0000-4000-8000-000000000001"
const B = "bbbbbbbb-0000-4000-8000-000000000002"
const I1 = "11111111-0000-4000-8000-000000000001"
const I2 = "22222222-0000-4000-8000-000000000002"
const P = "33333333-0000-4000-8000-000000000003"

const window = historyWindow("1h", Date.parse("2026-10-03T01:00:00Z"))

const bucket = (
  minute: number,
  patch: Partial<HistoryBucket> = {}
): HistoryBucket => ({
  from: new Date(window.fromMs + minute * 60_000).toISOString(),
  to: new Date(window.fromMs + (minute + 1) * 60_000).toISOString(),
  sourceNodeId: A,
  instanceId: I1,
  publicationId: P,
  kind: "neighbor",
  path: [A, B],
  linkId: "l1",
  transport: "native_udp",
  sampleCount: 2,
  success: 2,
  timeout: 0,
  failed: 0,
  meanSuccessDurationUs: 20_000,
  ...patch,
})

test("the window is fixed, ends on a second and buckets by its span", () => {
  const w = historyWindow("6h", Date.parse("2026-10-03T06:00:00.750Z"))
  assert.equal(w.toMs, Date.parse("2026-10-03T06:00:00Z"))
  assert.equal(w.toMs - w.fromMs, 6 * 3_600_000)
  assert.equal(w.bucketSeconds, 300)
})

test("buckets of one measurement from two instances merge into one point, the mean weighted by successes", () => {
  const [series] = bucketSeries(
    [
      bucket(3, {
        success: 1,
        sampleCount: 2,
        timeout: 1,
        meanSuccessDurationUs: 10_000,
      }),
      bucket(3, {
        instanceId: I2,
        success: 3,
        sampleCount: 3,
        meanSuccessDurationUs: 30_000,
      }),
    ],
    window
  )
  assert.ok(series)
  assert.equal(series.points.length, 1)
  const point = series.points[0]!
  assert.equal(point.index, 3)
  assert.equal(point.sampleCount, 5)
  assert.equal(point.success, 4)
  assert.equal(point.timeout, 1)
  assert.equal(point.meanUs, 25_000)
  assert.equal(series.instances, 2)
  assert.equal(unansweredRate(point), 0.2)
})

test("different links, transports or paths are different series", () => {
  const series = bucketSeries(
    [
      bucket(0),
      bucket(0, { linkId: "l2" }),
      bucket(0, { transport: "native_tcp" }),
    ],
    window
  )
  assert.equal(series.length, 3)
})

test("a missing bucket or one with no success breaks the line; nothing becomes zero", () => {
  const [series] = bucketSeries(
    [
      bucket(0),
      bucket(1),
      bucket(3),
      bucket(4, { success: 0, timeout: 2, meanSuccessDurationUs: null }),
      bucket(5),
    ],
    window
  )
  assert.ok(series)
  assert.deepEqual(
    lineRuns(series.points).map((run) => run.map((p) => p.index)),
    [[0, 1], [3], [5]]
  )
  assert.equal(series.totals.samples, 10)
  assert.equal(series.totals.timeout, 2)
  assert.equal(series.totals.meanUs, 20_000)
  assert.equal(series.points.find((p) => p.index === 4)?.meanUs, null)
})

test("no probes, no rate", () => {
  assert.equal(unansweredRate({ samples: 0, success: 0 }), null)
})

test("pruning before the window is left out; collection gaps keep their own interval", () => {
  const facts = coverageFacts(
    [
      {
        type: "pruned",
        sourceNodeId: A,
        reason: "retention",
        before: new Date(window.fromMs - 1000).toISOString(),
      },
      {
        type: "pruned",
        sourceNodeId: B,
        reason: "capacity",
        before: new Date(window.fromMs + 600_000).toISOString(),
      },
      {
        type: "collectionGap",
        sourceNodeId: A,
        instanceId: I1,
        eventId: "44444444-0000-4000-8000-000000000004",
        from: new Date(window.fromMs - 120_000).toISOString(),
        to: new Date(window.fromMs + 60_000).toISOString(),
        dropped: 7,
      },
    ],
    window
  )
  assert.deepEqual(
    facts.map((fact) => fact.type),
    ["collectionGap", "pruned"]
  )
  const gap = facts[0]
  assert.ok(gap?.type === "collectionGap")
  assert.equal(gap.dropped, 7)
  assert.equal(gap.fromMs, window.fromMs - 120_000)
})
