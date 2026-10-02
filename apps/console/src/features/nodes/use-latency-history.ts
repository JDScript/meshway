import {
  queryLatencyHistory,
  type LatencyHistoryRecord,
} from "@meshway/control-client"
import { useEffect, useState } from "react"

import { authed, SessionRejected } from "@/lib/api"

import {
  mergeRecords,
  nextWindow,
  PAGE_LIMIT,
  readWindow,
  SPAN_MS,
  type Coverage,
  type Span,
  type Window,
} from "./latency-model"

/** A visible chart re-reads this often; a hidden tab does not read. */
const REFRESH_MS = 45_000

export type LatencyState = {
  records: ReadonlyMap<string, LatencyHistoryRecord>
  /** The span on screen, ending at the last read. */
  range: Window | null
  coverage: Coverage
  expectedIntervalSeconds: number | null
  /** The last read stopped at the page budget. */
  partial: boolean
  /** When the last read finished; null before the first. */
  readAt: number | null
  /** The last read failed (the records are from before it). */
  failed: boolean
}

const INITIAL: LatencyState = {
  records: new Map(),
  range: null,
  coverage: { retentionFrom: null, trimmedBefore: null },
  expectedIntervalSeconds: null,
  partial: false,
  readAt: null,
  failed: false,
}

/**
 * One directed pair's history over the last `span`, read in fixed windows
 * (docs/spa-api-audit.md §历史分页): the first covers the span; each
 * refresh reads a new window from shortly before the end of the last one
 * read in full, with a fresh cursor, and merges by measurement. Reads stop
 * while the tab is hidden; one is in flight at a time. The caller keys the
 * component by pair and span, so a change starts over.
 */
export function useLatencyHistory({
  networkId,
  source,
  peer,
  span,
}: {
  networkId: string
  source: string
  peer: string
  span: Span
}): LatencyState {
  const [state, setState] = useState(INITIAL)

  useEffect(() => {
    let cancelled = false
    let busy = false
    let anchor: number | null = null
    let records: ReadonlyMap<string, LatencyHistoryRecord> = new Map()
    let coverage = INITIAL.coverage
    let lastRead = 0

    const read = async () => {
      if (busy || cancelled || document.visibilityState !== "visible") return
      busy = true
      lastRead = Date.now()
      const slice = nextWindow(span, Date.now(), anchor)
      try {
        const result = await readWindow(
          (after) =>
            authed((base, token) =>
              queryLatencyHistory(base, token, networkId, {
                source_node_id: source,
                peer_node_id: peer,
                from: new Date(slice.from).toISOString(),
                until: new Date(slice.until).toISOString(),
                after,
                limit: PAGE_LIMIT,
              })
            ),
          coverage
        )
        if (cancelled) return
        const range = { from: slice.until - SPAN_MS[span], until: slice.until }
        records = mergeRecords(
          records,
          result.records,
          { networkId, source, peer },
          range
        )
        coverage = result.coverage
        if (result.complete) anchor = slice.until
        setState({
          records,
          range,
          coverage,
          expectedIntervalSeconds: result.expectedIntervalSeconds,
          partial: !result.complete,
          readAt: Date.now(),
          failed: false,
        })
      } catch (error) {
        if (cancelled || error instanceof SessionRejected) return
        setState((current) => ({
          ...current,
          failed: true,
          readAt: Date.now(),
        }))
      } finally {
        busy = false
      }
    }

    void read()
    const timer = window.setInterval(() => void read(), REFRESH_MS)
    const onVisible = () => {
      if (
        document.visibilityState === "visible" &&
        Date.now() - lastRead >= REFRESH_MS
      )
        void read()
    }
    document.addEventListener("visibilitychange", onVisible)
    return () => {
      cancelled = true
      window.clearInterval(timer)
      document.removeEventListener("visibilitychange", onVisible)
    }
  }, [networkId, source, peer, span])

  return state
}
