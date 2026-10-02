"use client"

import type { LatencyHistoryRecord } from "@meshway/control-client"
import { useCallback, useEffect, useRef, useState } from "react"

import { readLatencyHistoryPageAction } from "@/lib/latency-actions"
import {
  readPages,
  windowRange,
  type LatencyPageResult,
  type LatencyQuery,
  type LatencyWindow,
} from "@/lib/latency-history-model"

/** Re-read the selected pair's window while it stays selected and the page is visible. */
const REFRESH_MS = 60_000

export type LatencyHistoryState = {
  /** The query the shown records answer; never another pair's or network's. */
  query: LatencyQuery | null
  records: LatencyHistoryRecord[]
  /** Continuation cursor when the page budget ended before the window did. */
  next: string | null
  expectedIntervalSeconds: number | null
  /** Time of the last successful read of `query`. */
  fetchedAt: number | null
  error: string | null
  loading: boolean
}

const empty: LatencyHistoryState = {
  query: null,
  records: [],
  next: null,
  expectedIntervalSeconds: null,
  fetchedAt: null,
  error: null,
  loading: false,
}

async function page(query: LatencyQuery, after: string | null): Promise<LatencyPageResult> {
  try {
    return await readLatencyHistoryPageAction(query.networkId, {
      sourceNodeId: query.sourceNodeId,
      peerNodeId: query.peerNodeId,
      from: query.from,
      until: query.until,
      after,
    })
  } catch {
    // The request itself failed (network, server restart): not an empty history.
    return { status: "error", message: "延迟历史读取失败：无法连接 Web 服务。" }
  }
}

/**
 * Bounded reads of one directed pair. Changing network, pair or window starts
 * a fresh read and fences every older response by sequence number; a refresh
 * of the same selection keeps the last good records (with their read time)
 * until it succeeds, and a failure leaves them visible as a dated cache.
 */
export function useLatencyHistory({
  networkId,
  sourceNodeId,
  peerNodeId,
  span,
  enabled,
}: {
  networkId: string
  sourceNodeId: string
  peerNodeId: string
  span: LatencyWindow
  enabled: boolean
}) {
  const [state, setState] = useState<LatencyHistoryState>(empty)
  const sequence = useRef(0)
  const stateRef = useRef(state)
  useEffect(() => {
    stateRef.current = state
  }, [state])
  const selection = `${networkId}|${sourceNodeId}|${peerNodeId}|${span}`
  const [shownSelection, setShownSelection] = useState(selection)
  if (shownSelection !== selection) {
    // A new selection never shows the previous one's records, even briefly.
    setShownSelection(selection)
    setState({ ...empty, loading: true })
  }

  const read = useCallback(
    (mode: "replace" | "continue") => {
      const seq = ++sequence.current
      const cancelled = () => seq !== sequence.current
      void (async () => {
        let query: LatencyQuery
        let after: string | null = null
        let base: LatencyHistoryRecord[] = []
        if (mode === "continue" && stateRef.current.query && stateRef.current.next) {
          query = stateRef.current.query
          after = stateRef.current.next
          base = stateRef.current.records
        } else {
          query = { networkId, sourceNodeId, peerNodeId, ...windowRange(span, Date.now()) }
        }
        const result = await readPages((cursor) => page(query, cursor), { after, cancelled })
        if (cancelled() || result.status === "cancelled") return
        if (result.status === "error") {
          setState((current) => ({ ...current, loading: false, error: result.message }))
          return
        }
        setState({
          query,
          records: [...base, ...result.records],
          next: result.next,
          expectedIntervalSeconds: result.expectedIntervalSeconds,
          fetchedAt: Date.now(),
          error: null,
          loading: false,
        })
      })()
    },
    [networkId, sourceNodeId, peerNodeId, span]
  )

  useEffect(() => {
    if (!enabled) return
    read("replace")
    let timer: number | undefined
    const schedule = () => {
      window.clearInterval(timer)
      if (document.visibilityState === "visible") timer = window.setInterval(() => read("replace"), REFRESH_MS)
    }
    const onVisibility = () => {
      if (document.visibilityState === "visible") read("replace")
      schedule()
    }
    schedule()
    document.addEventListener("visibilitychange", onVisibility)
    const owned = sequence
    return () => {
      owned.current += 1
      window.clearInterval(timer)
      document.removeEventListener("visibilitychange", onVisibility)
    }
  }, [enabled, read])

  return {
    state,
    refresh: useCallback(() => {
      setState((current) => ({ ...current, loading: true }))
      read("replace")
    }, [read]),
    readMore: useCallback(() => {
      setState((current) => ({ ...current, loading: true }))
      read("continue")
    }, [read]),
  }
}
