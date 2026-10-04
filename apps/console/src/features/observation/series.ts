import type { SegmentSample } from "./record.ts"

// The samples an observation delivered, as a chart and counts. A reply with
// its timing is a point; a timeout, a failed send or a window still open is
// a mark without a value; time with no sample is blank. Nothing becomes
// zero, and two path segments are never joined by a line.

export type SeriesPoint = {
  atMs: number
  segment: string
  outcome: "success" | "timeout" | "failed" | "pending"
  /** The round trip of a reply the node timed; null otherwise. */
  us: number | null
}

const outcomeOf = (entry: SegmentSample): SeriesPoint["outcome"] | null => {
  const probe = entry.sample.probe
  if (probe) return probe.outcome
  if (entry.sample.partial) return "pending"
  return null
}

/** One point per sample with a probe (or a window still open), in time order. */
export function seriesPoints(samples: readonly SegmentSample[]): SeriesPoint[] {
  const points: SeriesPoint[] = []
  for (const entry of samples) {
    const outcome = outcomeOf(entry)
    if (!outcome) continue
    const probe = entry.sample.probe
    const at = Date.parse(probe?.completedAt ?? entry.sample.windowEnd)
    if (!Number.isFinite(at)) continue
    points.push({
      atMs: at,
      segment: entry.segment,
      outcome,
      us:
        outcome === "success" && probe?.durationUs != null
          ? probe.durationUs
          : null,
    })
  }
  return points.sort((a, b) => a.atMs - b.atMs)
}

/**
 * The timed replies as runs to draw as lines: a run ends at anything that is
 * not a timed reply, at a new path segment, and where more than `gapMs`
 * passed without a sample.
 */
export function replyRuns(
  points: readonly SeriesPoint[],
  gapMs: number
): SeriesPoint[][] {
  const runs: SeriesPoint[][] = []
  let run: SeriesPoint[] = []
  for (const point of points) {
    const last = run.at(-1)
    const joins =
      point.us !== null &&
      last !== undefined &&
      last.segment === point.segment &&
      point.atMs - last.atMs <= gapMs
    if (!joins && run.length) {
      runs.push(run)
      run = []
    }
    if (point.us !== null) run.push(point)
  }
  if (run.length) runs.push(run)
  return runs
}

export type Totals = {
  samples: number
  replied: number
  timeout: number
  pending: number
  /** Probes the node could not send: not counted as unanswered. */
  failed: number
  minUs: number | null
  maxUs: number | null
  meanUs: number | null
  lastUs: number | null
}

/**
 * Counts over the samples. A sample's probe counts (`probeCounts`) are used
 * where it has them, else its one probe's outcome; failed sends come from
 * the probe. Timings are the timed replies'.
 */
export function totals(samples: readonly SegmentSample[]): Totals {
  const result: Totals = {
    samples: samples.length,
    replied: 0,
    timeout: 0,
    pending: 0,
    failed: 0,
    minUs: null,
    maxUs: null,
    meanUs: null,
    lastUs: null,
  }
  let sum = 0
  let timed = 0
  for (const { sample } of samples) {
    const counts = sample.probeCounts
    const probe = sample.probe
    if (counts) {
      result.replied += counts.repliedWithinDeadline
      result.timeout += counts.timeout
      result.pending += counts.pending
    } else if (probe) {
      if (probe.outcome === "success") result.replied += 1
      else if (probe.outcome === "timeout") result.timeout += 1
      else if (probe.outcome === "pending") result.pending += 1
    }
    if (probe?.outcome === "failed") result.failed += 1
    if (probe?.outcome === "success" && probe.durationUs != null) {
      const us = probe.durationUs
      sum += us
      timed += 1
      result.minUs = result.minUs === null ? us : Math.min(result.minUs, us)
      result.maxUs = result.maxUs === null ? us : Math.max(result.maxUs, us)
      result.lastUs = us
    }
  }
  if (timed) result.meanUs = sum / timed
  return result
}

/** 探测未回应率: timed out over probes with a final answer; null with none. */
export function unansweredRate(t: Pick<Totals, "replied" | "timeout">) {
  const final = t.replied + t.timeout
  return final ? t.timeout / final : null
}

/** How long without a sample breaks the line: a whole period past the next one due. */
export const gapFor = (intervalMs: number) => intervalMs * 2
