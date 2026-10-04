import type {
  CoverageReason,
  HistoryBucket,
  HistoryCoverage,
  ProbeKind,
} from "@meshway/control-client"

import type { HistoryWindow } from "./window"

// Probe buckets from the controller, as series a chart can draw: one per
// source, probe kind, path, link and transport. The controller also splits
// buckets by the source's instance and execution publication; those are
// the same measurement here and merge into one point per time bucket. A
// time bucket the controller returns no bucket for has no point: it was not
// measured (or not recorded), which is never drawn as zero.

export type SeriesPoint = {
  /** Bucket number from the window's start. */
  index: number
  fromMs: number
  toMs: number
  sampleCount: number
  success: number
  timeout: number
  failed: number
  /** Mean of the successful probes' durations; null without one. */
  meanUs: number | null
}

export type Series = {
  key: string
  sourceNodeId: string
  /** The path's last node. */
  peerNodeId: string
  kind: ProbeKind
  path: string[]
  linkId: string | null
  transport: string | null
  points: SeriesPoint[]
  totals: {
    samples: number
    success: number
    timeout: number
    failed: number
    /** Over the successful probes of the window; null without one. */
    meanUs: number | null
    /** The lowest and highest bucket means. */
    minUs: number | null
    maxUs: number | null
  }
  /** How many instances of the source the window's buckets came from. */
  instances: number
}

const KIND_ORDER: readonly ProbeKind[] = [
  "neighbor",
  "path",
  "connect",
  "controller",
]

const seriesKey = (bucket: HistoryBucket) =>
  JSON.stringify([
    bucket.sourceNodeId,
    bucket.kind,
    bucket.path,
    bucket.linkId ?? null,
    bucket.transport ?? null,
  ])

/**
 * The share of probes that got no successful reply (timed out or failed),
 * null without probes. Not a packet loss rate.
 */
export const unansweredRate = (point: {
  sampleCount?: number
  samples?: number
  success: number
}): number | null => {
  const total = point.sampleCount ?? point.samples ?? 0
  return total > 0 ? (total - point.success) / total : null
}

export function bucketSeries(
  buckets: readonly HistoryBucket[],
  window: HistoryWindow
): Series[] {
  const bucketMs = window.bucketSeconds * 1000
  const series = new Map<
    string,
    {
      first: HistoryBucket
      points: Map<number, SeriesPoint>
      instances: Set<string>
    }
  >()
  for (const bucket of buckets) {
    const key = seriesKey(bucket)
    let entry = series.get(key)
    if (!entry) {
      entry = { first: bucket, points: new Map(), instances: new Set() }
      series.set(key, entry)
    }
    entry.instances.add(bucket.instanceId)
    const fromMs = Date.parse(bucket.from)
    const index = Math.round((fromMs - window.fromMs) / bucketMs)
    const point = entry.points.get(index)
    const mean = bucket.meanSuccessDurationUs ?? null
    if (!point) {
      entry.points.set(index, {
        index,
        fromMs,
        toMs: Date.parse(bucket.to),
        sampleCount: bucket.sampleCount,
        success: bucket.success,
        timeout: bucket.timeout,
        failed: bucket.failed,
        meanUs: mean,
      })
      continue
    }
    // The controller accepts a successful probe only with its duration, so a
    // bucket's mean covers exactly its `success` probes: weighting by them
    // is exact.
    const weighted =
      (point.meanUs ?? 0) * point.success + (mean ?? 0) * bucket.success
    point.sampleCount += bucket.sampleCount
    point.success += bucket.success
    point.timeout += bucket.timeout
    point.failed += bucket.failed
    point.meanUs = point.success > 0 ? weighted / point.success : null
  }
  return [...series.entries()]
    .map(([key, { first, points, instances }]): Series => {
      const sorted = [...points.values()].sort((a, b) => a.index - b.index)
      let samples = 0
      let success = 0
      let timeout = 0
      let failed = 0
      let sum = 0
      let minUs: number | null = null
      let maxUs: number | null = null
      for (const point of sorted) {
        samples += point.sampleCount
        success += point.success
        timeout += point.timeout
        failed += point.failed
        if (point.meanUs !== null) {
          sum += point.meanUs * point.success
          minUs = minUs === null ? point.meanUs : Math.min(minUs, point.meanUs)
          maxUs = maxUs === null ? point.meanUs : Math.max(maxUs, point.meanUs)
        }
      }
      return {
        key,
        sourceNodeId: first.sourceNodeId,
        peerNodeId: first.path.at(-1) ?? first.sourceNodeId,
        kind: first.kind,
        path: first.path,
        linkId: first.linkId ?? null,
        transport: first.transport ?? null,
        points: sorted,
        totals: {
          samples,
          success,
          timeout,
          failed,
          meanUs: success > 0 ? sum / success : null,
          minUs,
          maxUs,
        },
        instances: instances.size,
      }
    })
    .sort(
      (a, b) =>
        KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) ||
        a.key.localeCompare(b.key)
    )
}

/**
 * Runs of points a line may join: neighbouring buckets that both have a
 * successful mean. A missing bucket, or one where nothing succeeded, ends a
 * run.
 */
export function lineRuns(points: readonly SeriesPoint[]): SeriesPoint[][] {
  const runs: SeriesPoint[][] = []
  let run: SeriesPoint[] = []
  for (const point of points) {
    const previous = run.at(-1)
    if (
      point.meanUs === null ||
      (previous !== undefined && point.index !== previous.index + 1)
    ) {
      if (run.length) runs.push(run)
      run = []
    }
    if (point.meanUs !== null) run.push(point)
  }
  if (run.length) runs.push(run)
  return runs
}

/** What the window does not cover, deduplicated, earliest first. */
export type CoverageFact =
  | {
      type: "pruned"
      sourceNodeId: string
      reason: CoverageReason
      beforeMs: number
    }
  | {
      type: "collectionGap"
      sourceNodeId: string
      fromMs: number
      toMs: number
      dropped: number
    }

/**
 * Pruning matters only when it reaches into the window; a collection gap
 * is as the source reported it (its whole interval, its drop count).
 */
export function coverageFacts(
  coverage: readonly HistoryCoverage[],
  window: HistoryWindow
): CoverageFact[] {
  const facts = new Map<string, CoverageFact>()
  for (const item of coverage) {
    if (item.type === "pruned") {
      const beforeMs = Date.parse(item.before)
      if (beforeMs <= window.fromMs) continue
      facts.set(`p|${item.sourceNodeId}|${item.reason}`, {
        type: "pruned",
        sourceNodeId: item.sourceNodeId,
        reason: item.reason,
        beforeMs,
      })
    } else {
      facts.set(`g|${item.sourceNodeId}|${item.eventId}`, {
        type: "collectionGap",
        sourceNodeId: item.sourceNodeId,
        fromMs: Date.parse(item.from),
        toMs: Date.parse(item.to),
        dropped: item.dropped,
      })
    }
  }
  const start = (fact: CoverageFact) =>
    fact.type === "pruned" ? window.fromMs : fact.fromMs
  return [...facts.values()].sort((a, b) => start(a) - start(b))
}
