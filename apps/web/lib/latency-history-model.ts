import type { ConnectionTransport, LatencyFailureReason, LatencyHistoryRecord, LatencyHistoryResponse, LatencyOutcome } from "@meshway/control-client"

// Directed authenticated-neighbor RTT history (docs/latency-history.md).
// Every record is historical evidence, never present connectivity. Only the
// generated response fields are used; nothing is aggregated server-side, so
// gaps, timeouts and failures stay distinct from successful measurements.

export const LATENCY_WINDOWS = [
  { value: "15m", label: "15 分钟", ms: 15 * 60_000 },
  { value: "1h", label: "1 小时", ms: 60 * 60_000 },
  { value: "6h", label: "6 小时", ms: 6 * 60 * 60_000 },
  { value: "24h", label: "24 小时", ms: 24 * 60 * 60_000 },
] as const

export type LatencyWindow = (typeof LATENCY_WINDOWS)[number]["value"]

export const DEFAULT_WINDOW: LatencyWindow = "1h"
/** One query page; the API caps it at 1,000. */
export const PAGE_LIMIT = 1000
/** Pages per read: bounds requests and points (24 h at 30 s is ~2,900 events). */
export const PAGE_BUDGET = 6

export function windowMs(window: LatencyWindow): number {
  return LATENCY_WINDOWS.find((entry) => entry.value === window)?.ms ?? LATENCY_WINDOWS[1].ms
}

/** Half-open `[from, until)`; whole seconds keep repeated reads of one window identical. */
export function windowRange(window: LatencyWindow, now: number): { from: string; until: string } {
  const until = Math.floor(now / 1000) * 1000
  return { from: new Date(until - windowMs(window)).toISOString(), until: new Date(until).toISOString() }
}

/** What a read asks for; a response is accepted only for exactly this key. */
export type LatencyQuery = { networkId: string; sourceNodeId: string; peerNodeId: string; from: string; until: string }

export function queryKey(query: LatencyQuery): string {
  return [query.networkId, query.sourceNodeId, query.peerNodeId, query.from, query.until].join("|")
}

export type LatencyPageResult =
  | { status: "loaded"; page: LatencyHistoryResponse }
  | { status: "error"; message: string }

/**
 * Reads pages by the receive-order cursor until the API reports no more or
 * the page budget is spent; then the read is `truncated` with the cursor to
 * continue from. Never claims a complete window it did not read.
 */
export async function readPages(
  readPage: (after: string | null) => Promise<LatencyPageResult>,
  options: { budget?: number; after?: string | null; cancelled?: () => boolean } = {}
): Promise<
  | { status: "loaded"; records: LatencyHistoryRecord[]; next: string | null; pages: number; expectedIntervalSeconds: number | null }
  | { status: "error"; message: string; records: LatencyHistoryRecord[] }
  | { status: "cancelled" }
> {
  const budget = options.budget ?? PAGE_BUDGET
  const records: LatencyHistoryRecord[] = []
  let after = options.after ?? null
  let interval: number | null = null
  for (let pages = 0; pages < budget; ) {
    const result = await readPage(after)
    if (options.cancelled?.()) return { status: "cancelled" }
    if (result.status === "error") return { status: "error", message: result.message, records }
    pages += 1
    records.push(...result.page.records)
    interval = result.page.expected_interval_seconds
    if (result.page.next === null) return { status: "loaded", records, next: null, pages, expectedIntervalSeconds: interval }
    after = result.page.next
    if (pages === budget) return { status: "loaded", records, next: after, pages, expectedIntervalSeconds: interval }
  }
  return { status: "loaded", records, next: after, pages: 0, expectedIntervalSeconds: interval }
}

export type SuccessPoint = { id: string; at: number; ms: number; segment: string }
export type LatencyEvent = {
  id: string
  at: number
  outcome: Exclude<LatencyOutcome, "success">
  reason: LatencyFailureReason | null
}
export type SegmentChange = { at: number; changed: string[] }

export type Segment = {
  key: string
  transport: ConnectionTransport
  listenerId: string
  candidateId: string | null
  initiatorNodeId: string
  runtimeEpoch: number
  revision: number
  samplerGeneration: string
  /** Consecutive successes of this segment; a new run starts after an unrecorded stretch. */
  runs: SuccessPoint[][]
}

export type LatencyHistory = {
  successes: SuccessPoint[]
  events: LatencyEvent[]
  segments: Segment[]
  changes: SegmentChange[]
  /** Records outside the query (network, pair, window) or duplicated, dropped. */
  ignored: number
  stats: { success: number; timeout: number; failed: number; minMs: number | null; medianMs: number | null; maxMs: number | null }
}

/** Microseconds to milliseconds for display; precision follows magnitude, never rounds a value to 0. */
export function formatMs(ms: number): string {
  if (ms < 0.01) return "<0.01 ms"
  if (ms < 1) return `${ms.toFixed(2)} ms`
  if (ms < 10) return `${ms.toFixed(1)} ms`
  return `${Math.round(ms)} ms`
}

export const failureReasonLabels: Record<LatencyFailureReason, string> = {
  channel_unavailable: "通道不可用",
  authentication_failed: "认证失败",
  local_send_failed: "本地发送失败",
}

function segmentFields(record: LatencyHistoryRecord) {
  const sample = record.sample
  const association = sample.channel_associations[0]
  if (!association) return null
  return {
    key: [
      record.network_id,
      record.source_node_id,
      sample.peer_node_id,
      sample.direction,
      sample.runtime_epoch,
      sample.revision,
      sample.digest,
      sample.sampler_generation,
      association.transport,
      association.initiator_node_id,
      association.listener_id,
      association.candidate_id ?? "",
    ].join("|"),
    transport: association.transport,
    listenerId: association.listener_id,
    candidateId: association.candidate_id,
    initiatorNodeId: association.initiator_node_id,
    runtimeEpoch: sample.runtime_epoch,
    revision: sample.revision,
    digest: sample.digest,
    samplerGeneration: sample.sampler_generation,
  }
}

const changeLabels: [keyof Omit<NonNullable<ReturnType<typeof segmentFields>>, "key">, string][] = [
  ["transport", "承载"],
  ["listenerId", "监听"],
  ["candidateId", "拨号候选"],
  ["initiatorNodeId", "发起方"],
  ["runtimeEpoch", "运行代际"],
  ["revision", "配置版本"],
  ["digest", "配置摘要"],
  ["samplerGeneration", "采样代际"],
]

/**
 * Builds the chart model for one directed pair. Successes are ordered by
 * their own `measured_at`, deduplicated by `measurement_id`, and grouped by
 * the full segment key; a line never joins two segments, never crosses a
 * timeout / failure, and a stretch longer than `breakAfterMs` without a
 * record starts a new run (unrecorded, not offline). Timeouts and failures carry no channel association, so they are
 * pair events, never attached to a carrier line.
 */
export function buildHistory(records: LatencyHistoryRecord[], query: LatencyQuery, breakAfterMs: number): LatencyHistory {
  const from = Date.parse(query.from)
  const until = Date.parse(query.until)
  const seen = new Set<string>()
  let ignored = 0
  const kept: { record: LatencyHistoryRecord; at: number }[] = []
  for (const record of records) {
    const at = Date.parse(record.sample.measured_at)
    const inScope =
      record.network_id === query.networkId &&
      record.source_node_id === query.sourceNodeId &&
      record.sample.peer_node_id === query.peerNodeId &&
      record.sample.direction === "source_to_peer" &&
      Number.isFinite(at) && at >= from && at < until
    if (!inScope || seen.has(record.sample.measurement_id)) {
      ignored += 1
      continue
    }
    seen.add(record.sample.measurement_id)
    kept.push({ record, at })
  }
  kept.sort((a, b) => a.at - b.at || a.record.sample.measurement_id.localeCompare(b.record.sample.measurement_id))

  const successes: SuccessPoint[] = []
  const events: LatencyEvent[] = []
  const segments = new Map<string, Segment>()
  const changes: SegmentChange[] = []
  let previous: ReturnType<typeof segmentFields> = null
  // A timeout or failure between two successes is a measured miss: the line breaks there.
  let interrupted = false
  for (const { record, at } of kept) {
    const sample = record.sample
    if (sample.outcome !== "success") {
      events.push({ id: sample.measurement_id, at, outcome: sample.outcome, reason: sample.failure_reasons[0] ?? null })
      interrupted = true
      continue
    }
    const fields = segmentFields(record)
    if (!fields || sample.rtt_us === null) {
      ignored += 1
      continue
    }
    const point: SuccessPoint = { id: sample.measurement_id, at, ms: sample.rtt_us / 1000, segment: fields.key }
    successes.push(point)
    if (previous && previous.key !== fields.key) {
      const last = previous
      changes.push({ at, changed: changeLabels.filter(([field]) => last[field] !== fields[field]).map(([, label]) => label) })
    }
    let segment = segments.get(fields.key)
    if (!segment) {
      segment = {
        key: fields.key,
        transport: fields.transport,
        listenerId: fields.listenerId,
        candidateId: fields.candidateId,
        initiatorNodeId: fields.initiatorNodeId,
        runtimeEpoch: fields.runtimeEpoch,
        revision: fields.revision,
        samplerGeneration: fields.samplerGeneration,
        runs: [],
      }
      segments.set(fields.key, segment)
    }
    const run = segment.runs.at(-1)
    const lastPoint = run?.at(-1)
    // A different segment in between, or a long unrecorded stretch, ends the run.
    const contiguous = run && lastPoint && !interrupted && previous?.key === fields.key && at - lastPoint.at <= breakAfterMs
    if (contiguous) run.push(point)
    else segment.runs.push([point])
    previous = fields
    interrupted = false
  }

  const values = successes.map((point) => point.ms).sort((a, b) => a - b)
  const median = values.length === 0 ? null : values.length % 2 === 1 ? (values[(values.length - 1) / 2] ?? null) : ((values[values.length / 2 - 1] ?? 0) + (values[values.length / 2] ?? 0)) / 2
  return {
    successes,
    events,
    segments: [...segments.values()],
    changes,
    ignored,
    stats: {
      success: successes.length,
      timeout: events.filter((event) => event.outcome === "timeout").length,
      failed: events.filter((event) => event.outcome === "failed").length,
      minMs: values[0] ?? null,
      medianMs: median,
      maxMs: values.at(-1) ?? null,
    },
  }
}

/** Gap threshold for drawing: generous, because >64 neighbors rotate and busy slots skip. */
export function breakAfter(expectedIntervalSeconds: number | null): number {
  return Math.max(4 * (expectedIntervalSeconds ?? 30), 120) * 1000
}
