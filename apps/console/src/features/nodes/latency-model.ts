import type {
  ConnectionTransport,
  LatencyFailureReason,
  LatencyHistoryRecord,
  LatencyHistoryResponse,
} from "@meshway/control-client"

// Directed authenticated-neighbour RTT history (docs/latency-history.md,
// docs/spa-api-audit.md §历史分页). Records are raw events: nothing is
// aggregated or filled in. A timeout or failure carries no RTT and no
// channel, so it is a mark on the pair, never a point on a line; a stretch
// without records is a gap (unrecorded, not offline, not zero).

export const SPAN_MS = { "1h": 3_600_000, "6h": 21_600_000, "24h": 86_400_000 }
export type Span = keyof typeof SPAN_MS

/** One page; the API allows 1–1000. */
export const PAGE_LIMIT = 1000
/** Pages per window read: 24 h at 30 s is ~2,900 events. */
export const PAGE_BUDGET = 6
/** A refresh re-reads this far before the last complete window's end (measurements may arrive up to 120 s late). */
export const OVERLAP_MS = 130_000

export type Window = { from: number; until: number }

/**
 * The next window to read. The first covers the whole span; a refresh
 * covers from shortly before the last fully read window ended, so late
 * arrivals are caught. A cursor never carries over between windows.
 */
export function nextWindow(
  span: Span,
  now: number,
  anchor: number | null
): Window {
  const until = Math.floor(now / 1000) * 1000
  const floor = until - SPAN_MS[span]
  return {
    from: anchor === null ? floor : Math.max(floor, anchor - OVERLAP_MS),
    until,
  }
}

export type Coverage = {
  /** The controller's retention floor. */
  retentionFrom: number | null
  /** Some earlier events of the source (all peers) were evicted for capacity. */
  trimmedBefore: number | null
}

const later = (a: number | null, b: number | null) =>
  a === null ? b : b === null ? a : Math.max(a, b)
const time = (iso: string | null) => {
  if (iso === null) return null
  const at = Date.parse(iso)
  return Number.isNaN(at) ? null : at
}

export type WindowRead = {
  records: LatencyHistoryRecord[]
  /** Every page of the window was read. */
  complete: boolean
  coverage: Coverage
  expectedIntervalSeconds: number | null
}

/**
 * Reads one fixed window by its receive-order cursor, until the controller
 * reports no more or the page budget is spent (then it is incomplete).
 * Each page reports the coverage at its own time; the merge only grows
 * more conservative.
 */
export async function readWindow(
  readPage: (after: string | null) => Promise<LatencyHistoryResponse>,
  coverage: Coverage,
  budget = PAGE_BUDGET
): Promise<WindowRead> {
  const records: LatencyHistoryRecord[] = []
  let after: string | null = null
  let merged = coverage
  let interval: number | null = null
  for (let pages = 0; pages < budget; pages += 1) {
    const page = await readPage(after)
    records.push(...page.records)
    interval = page.expected_interval_seconds
    merged = {
      retentionFrom: later(merged.retentionFrom, time(page.retention_from)),
      trimmedBefore: later(
        merged.trimmedBefore,
        time(page.capacity_trimmed_before)
      ),
    }
    if (page.next === null)
      return {
        records,
        complete: true,
        coverage: merged,
        expectedIntervalSeconds: interval,
      }
    after = page.next
  }
  return {
    records,
    complete: false,
    coverage: merged,
    expectedIntervalSeconds: interval,
  }
}

/** Keeps what belongs to the pair and to `range` (the span on screen), one record per measurement. */
export function mergeRecords(
  kept: ReadonlyMap<string, LatencyHistoryRecord>,
  incoming: readonly LatencyHistoryRecord[],
  pair: { networkId: string; source: string; peer: string },
  range: Window
): Map<string, LatencyHistoryRecord> {
  const next = new Map<string, LatencyHistoryRecord>()
  const keep = (record: LatencyHistoryRecord) => {
    const at = Date.parse(record.sample.measured_at)
    if (
      record.network_id === pair.networkId &&
      record.source_node_id === pair.source &&
      record.sample.peer_node_id === pair.peer &&
      record.sample.direction === "source_to_peer" &&
      Number.isFinite(at) &&
      at >= range.from &&
      at < range.until
    )
      next.set(record.sample.measurement_id, record)
  }
  for (const record of kept.values()) keep(record)
  for (const record of incoming) keep(record)
  return next
}

export type Carrier = {
  transport: ConnectionTransport
  listenerId: string
  candidateId: string | null
  initiatorNodeId: string
}

export type ChartPoint = {
  at: number
  /** Milliseconds; null breaks the line. */
  rtt: number | null
  carrier: Carrier | null
}

export type PairEvent = {
  at: number
  outcome: "timeout" | "failed"
  reason: LatencyFailureReason | null
}

export type History = {
  points: ChartPoint[]
  events: PairEvent[]
  /** Where the measured channel changed (carrier, runtime, configuration or sampler). */
  changes: number[]
  stats: {
    success: number
    timeout: number
    failed: number
    min: number | null
    median: number | null
    max: number | null
  }
}

const segmentKey = (record: LatencyHistoryRecord) => {
  const sample = record.sample
  const channel = sample.channel_associations[0]
  return [
    sample.runtime_epoch,
    sample.revision,
    sample.digest,
    sample.sampler_generation,
    channel?.transport,
    channel?.initiator_node_id,
    channel?.listener_id,
    channel?.candidate_id ?? "",
  ].join("|")
}

/**
 * The chart for one directed pair within `window`, ordered by measurement
 * time. A line never joins two segments, never crosses a timeout or a
 * failure, and breaks after `breakAfterMs` without a record.
 */
export function buildHistory(
  records: Iterable<LatencyHistoryRecord>,
  window: Window,
  breakAfterMs: number
): History {
  const inWindow = [...records]
    .map((record) => ({ record, at: Date.parse(record.sample.measured_at) }))
    .filter(({ at }) => at >= window.from && at < window.until)
    .sort(
      (a, b) =>
        a.at - b.at ||
        a.record.sample.measurement_id.localeCompare(
          b.record.sample.measurement_id
        )
    )

  const points: ChartPoint[] = []
  const events: PairEvent[] = []
  const changes: number[] = []
  const rtts: number[] = []
  const previous = { at: 0, key: "" }
  let lastKey: string | null = null
  let broken = true

  for (const { record, at } of inWindow) {
    const sample = record.sample
    const channel = sample.channel_associations[0]
    if (sample.outcome !== "success" || sample.rtt_us === null || !channel) {
      if (sample.outcome !== "success")
        events.push({
          at,
          outcome: sample.outcome,
          reason: sample.failure_reasons[0] ?? null,
        })
      broken = true
      continue
    }
    const key = segmentKey(record)
    if (lastKey !== null && key !== lastKey) changes.push(at)
    if (
      points.length > 0 &&
      (broken || at - previous.at > breakAfterMs || key !== previous.key)
    )
      points.push({ at: previous.at + 1, rtt: null, carrier: null })
    const rtt = sample.rtt_us / 1000
    points.push({
      at,
      rtt,
      carrier: {
        transport: channel.transport,
        listenerId: channel.listener_id,
        candidateId: channel.candidate_id,
        initiatorNodeId: channel.initiator_node_id,
      },
    })
    rtts.push(rtt)
    previous.at = at
    previous.key = key
    lastKey = key
    broken = false
  }

  const sorted = [...rtts].sort((a, b) => a - b)
  const middle = Math.floor(sorted.length / 2)
  const median =
    sorted.length === 0
      ? null
      : sorted.length % 2 === 1
        ? (sorted[middle] ?? null)
        : ((sorted[middle - 1] ?? 0) + (sorted[middle] ?? 0)) / 2
  return {
    points,
    events,
    changes,
    stats: {
      success: rtts.length,
      timeout: events.filter((event) => event.outcome === "timeout").length,
      failed: events.filter((event) => event.outcome === "failed").length,
      min: sorted[0] ?? null,
      median,
      max: sorted.at(-1) ?? null,
    },
  }
}

/** A line breaks after this long without a record: 2.5 expected intervals, at least a minute. */
export const breakAfter = (expectedIntervalSeconds: number | null) =>
  Math.max(60_000, (expectedIntervalSeconds ?? 30) * 2_500)

/** Milliseconds for display; never rounds a real measurement down to 0. */
export function msText(ms: number): string {
  if (ms < 0.01) return "<0.01"
  if (ms < 1) return ms.toFixed(2)
  if (ms < 10) return ms.toFixed(1)
  return String(Math.round(ms))
}
