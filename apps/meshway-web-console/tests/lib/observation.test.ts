import assert from "node:assert/strict"
import type {
  AggregatedPath,
  ObservationSample,
  ObservationTask,
} from "@meshway/control-client"
import { test } from "vitest"

import { canObserveFrom, pathChoices } from "@/lib/observation/choices"
import { leasePlan, retryPlan } from "@/lib/observation/lease"
import {
  emptyRecord,
  foldTask,
  removeRecord,
  standing,
  type ObservationRecord,
} from "@/lib/observation/record"
import {
  gapFor,
  replyRuns,
  seriesPoints,
  totals,
  unansweredRate,
} from "@/lib/observation/series"

// Typed test data only: the panel shows what the controller sent.

const A = "aaaaaaaa-0000-4000-8000-000000000001"
const B = "bbbbbbbb-0000-4000-8000-000000000002"
const C = "cccccccc-0000-4000-8000-000000000003"
const S1 = "51515151-0000-4000-8000-000000000001"
const S2 = "52525252-0000-4000-8000-000000000002"
const P1 = "70707070-0000-4000-8000-000000000001"
const P2 = "70707070-0000-4000-8000-000000000002"
const T0 = Date.parse("2026-10-03T12:00:00Z")
const iso = (ms: number) => new Date(ms).toISOString()
const mid = (n: number) =>
  `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`

const task = (patch: Partial<ObservationTask> = {}): ObservationTask => ({
  id: "0b0b0b0b-0000-4000-8000-000000000001",
  subscriptionId: S1,
  sourceNodeId: A,
  targetNodeId: B,
  service: { type: "icmpEcho" },
  selection: { mode: "auto" },
  configuration: {
    revision: 3,
    executionPublication: P1,
    digest: "d".repeat(64),
  },
  instanceId: "1e1e1e1e-0000-4000-8000-000000000001",
  intervalMs: 1000,
  metrics: ["rtt", "probeLoss"],
  unsupportedMetrics: [],
  phase: "preparing",
  path: null,
  pathSegment: null,
  latest: null,
  expiresAt: iso(T0 + 30_000),
  subscriberExpiresAt: iso(T0 + 30_000),
  renewAfter: iso(T0 + 10_000),
  ...patch,
})

const sample = (
  n: number,
  outcome: "success" | "timeout" | "failed" | "pending",
  patch: Partial<ObservationSample> = {}
): ObservationSample => {
  const start = T0 + n * 1000
  return {
    measurementId: mid(n),
    partial: outcome === "pending",
    version: 1,
    windowStart: iso(start),
    windowEnd: iso(start + 500),
    probe: {
      kind: "path",
      measurementId: mid(n),
      outcome,
      path: [A, B],
      publicationId: P1,
      startedAt: iso(start),
      completedAt: outcome === "pending" ? null : iso(start + 400),
      durationUs: outcome === "success" ? 40_000 + n : null,
    },
    probeCounts: {
      sent: 1,
      repliedWithinDeadline: outcome === "success" ? 1 : 0,
      timeout: outcome === "timeout" ? 1 : 0,
      pending: outcome === "pending" ? 1 : 0,
    },
    ...patch,
  }
}

const running = (
  latest: ObservationSample,
  patch: Partial<ObservationTask> = {}
) => task({ phase: "running", path: [A, B], pathSegment: 1, latest, ...patch })

test("a task's latest sample joins the series once", () => {
  let record: ObservationRecord = foldTask(emptyRecord, task())
  assert.equal(record.samples.length, 0)
  record = foldTask(record, running(sample(1, "success")))
  // The same sample again, as `observation.status` repeats it.
  record = foldTask(record, running(sample(1, "success")))
  record = foldTask(record, running(sample(2, "timeout")))
  assert.deepEqual(
    record.samples.map((s) => s.sample.measurementId),
    [mid(1), mid(2)]
  )
  assert.equal(record.task?.phase, "running")
})

test("a partial window is replaced by its later version, never by an earlier one", () => {
  let record = foldTask(emptyRecord, running(sample(1, "pending")))
  record = foldTask(record, running(sample(1, "success", { version: 2 })))
  assert.equal(record.samples.length, 1)
  assert.equal(record.samples[0]!.sample.probe?.outcome, "success")
  record = foldTask(record, running(sample(1, "pending")))
  assert.equal(record.samples[0]!.sample.probe?.outcome, "success")
})

test("samples keep their segment: a new publication or path segment is another", () => {
  let record = foldTask(emptyRecord, running(sample(1, "success")))
  record = foldTask(
    record,
    running(sample(2, "success"), { path: [A, C, B], pathSegment: 2 })
  )
  record = foldTask(
    record,
    running(sample(3, "success"), {
      configuration: {
        revision: 3,
        executionPublication: P2,
        digest: "e".repeat(64),
      },
    })
  )
  assert.deepEqual(
    record.samples.map((s) => [s.segment, s.path.length]),
    [
      [`${P1}/1`, 2],
      [`${P1}/2`, 3],
      [`${P2}/1`, 2],
    ]
  )
})

test("a terminal view keeps what was measured", () => {
  let record = foldTask(emptyRecord, running(sample(1, "success")))
  record = foldTask(record, task({ phase: "cancelled", latest: null }))
  assert.equal(record.samples.length, 1)
  assert.deepEqual(standing(record, T0), { type: "ended", phase: "cancelled" })
})

test("another subscription's view and views after removal change nothing", () => {
  const record = foldTask(emptyRecord, task())
  assert.equal(
    foldTask(record, running(sample(1, "success"), { subscriptionId: S2 })),
    record
  )
  const removed = removeRecord(record)
  assert.equal(removed.removed, true)
  assert.equal(foldTask(removed, running(sample(1, "success"))), removed)
  assert.equal(standing(removed, T0).type, "ended")
})

test("the series is bounded, keeping the latest", () => {
  const total = 5_000
  let record = emptyRecord
  for (let n = 0; n < total; n++)
    record = foldTask(record, running(sample(n, "success")))
  const kept = record.samples.length
  assert.ok(kept > 0 && kept < total, `${kept} kept`)
  assert.equal(record.samples[0]!.sample.measurementId, mid(total - kept))
  assert.equal(record.samples.at(-1)!.sample.measurementId, mid(total - 1))
})

test("a lease that ran out is lapsed before the controller says so", () => {
  const record = foldTask(emptyRecord, task())
  assert.deepEqual(standing(record, T0 + 29_999), {
    type: "active",
    phase: "preparing",
  })
  assert.deepEqual(standing(record, T0 + 30_000), { type: "lapsed" })
  assert.deepEqual(standing(emptyRecord, T0), { type: "unknown" })
})

test("renewal waits for renewAfter and stops at the lease's end", () => {
  assert.deepEqual(leasePlan(task(), T0), { type: "renew", inMs: 10_250 })
  assert.deepEqual(leasePlan(task(), T0 + 12_000), { type: "renew", inMs: 0 })
  assert.deepEqual(leasePlan(task(), T0 + 30_000), { type: "lapsed" })
  assert.deepEqual(leasePlan(task({ phase: "unsupported" }), T0), {
    type: "none",
  })
  // A renewAfter at or past the lease's end still renews before it.
  assert.deepEqual(
    leasePlan(task({ renewAfter: iso(T0 + 30_000) }), T0 + 20_000),
    { type: "renew", inMs: 9_750 }
  )
})

test("a renewal refused as early is tried again while the lease lasts", () => {
  assert.deepEqual(retryPlan(task(), T0 + 10_000), {
    type: "renew",
    inMs: 1_000,
  })
  assert.deepEqual(retryPlan(task(), T0 + 29_500), {
    type: "renew",
    inMs: 250,
  })
  assert.deepEqual(retryPlan(task(), T0 + 29_800), { type: "lapsed" })
  assert.deepEqual(retryPlan(task({ phase: "failed" }), T0), { type: "none" })
})

test("points: timed replies have values; timeouts, failures and open windows do not", () => {
  const record = [
    sample(1, "success"),
    sample(2, "timeout"),
    sample(3, "failed", { probeCounts: null }),
    sample(4, "pending"),
    sample(5, "success", {
      probe: { ...sample(5, "success").probe!, durationUs: null },
    }),
  ].reduce((r, s) => foldTask(r, running(s)), emptyRecord)
  const points = seriesPoints(record.samples)
  assert.deepEqual(
    points.map((p) => [p.outcome, p.us]),
    [
      ["success", 40_001],
      ["timeout", null],
      ["failed", null],
      ["pending", null],
      ["success", null],
    ]
  )
})

test("lines break at a timeout, a missing sample and a new segment", () => {
  let record = emptyRecord
  for (const n of [1, 2, 4, 5, 9, 10])
    record = foldTask(
      record,
      running(sample(n, n === 4 ? "timeout" : "success"))
    )
  record = foldTask(
    record,
    running(sample(11, "success"), { path: [A, C, B], pathSegment: 2 })
  )
  const runs = replyRuns(seriesPoints(record.samples), gapFor(1000))
  assert.deepEqual(
    runs.map((run) => run.length),
    // 1–2, (timeout at 4) 5, (no samples 6–8) 9–10, (new segment) 11
    [2, 1, 2, 1]
  )
})

test("counts use probe counts, keep failed sends apart, and never invent a rate", () => {
  let record = emptyRecord
  for (const [n, outcome] of [
    [1, "success"],
    [2, "success"],
    [3, "timeout"],
    [4, "failed"],
    [5, "pending"],
  ] as const)
    record = foldTask(
      record,
      running(
        sample(n, outcome, outcome === "failed" ? { probeCounts: null } : {})
      )
    )
  const t = totals(record.samples)
  assert.deepEqual(
    [t.samples, t.replied, t.timeout, t.pending, t.failed],
    [5, 2, 1, 1, 1]
  )
  assert.equal(t.minUs, 40_001)
  assert.equal(t.maxUs, 40_002)
  assert.equal(t.meanUs, 40_001.5)
  assert.equal(t.lastUs, 40_002)
  assert.equal(unansweredRate(t), 1 / 3)
  assert.equal(unansweredRate(totals([])), null)
  assert.equal(totals([]).meanUs, null)
})

test("explicit paths: direct first, then reported and observed ones between the two nodes", () => {
  const reported = (nodes: string[]): AggregatedPath => ({
    pathId: nodes.join(),
    reporter: {
      instanceId: "1e1e1e1e-0000-4000-8000-000000000001",
      nodeId: A,
      reportSequence: 1,
    },
    state: {
      evidence: "selected",
      expiresAt: iso(T0),
      nodes,
      observedAt: iso(T0),
      publicationId: P1,
      reporterRole: "origin",
    },
  })
  const choices = pathChoices(
    A,
    B,
    [reported([A, C, B]), reported([B, A]), reported([A, C, C, B])],
    [
      [A, C, B],
      [A, B],
    ]
  )
  assert.deepEqual(choices, [
    [A, B],
    [A, C, B],
  ])
})

test("who may start: owners and admins anywhere, members from their own User devices", () => {
  const user = { type: "user" as const, user: "me@example.com" }
  const tag = { type: "tag" as const, tags: ["tag:server"] }
  assert.equal(canObserveFrom("owner", null, tag), true)
  assert.equal(canObserveFrom("admin", "a@example.com", user), true)
  assert.equal(canObserveFrom("member", "me@example.com", user), true)
  assert.equal(canObserveFrom("member", "other@example.com", user), false)
  assert.equal(canObserveFrom("member", "me@example.com", tag), false)
  assert.equal(canObserveFrom(null, "me@example.com", user), false)
})
