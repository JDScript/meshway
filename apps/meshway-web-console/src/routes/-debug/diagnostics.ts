import type { QueryClient } from "@tanstack/react-query"

import { clockOffset } from "@/lib/debug/clock"
import { unsettledAttempts } from "@/state/attempts"
import { traceNow, type StreamEntry } from "@/state/debug-trace"
import type { Connection } from "@/state/live"

// What the panel copies for a report: when and where, the clock, each
// stream's standing and entries, the requests and the cache's reads. Never
// an event's data, a draft or a credential: an event keeps its type, ID,
// size, effects and the reads it touched.

const iso = (at: number) => new Date(at).toISOString()

function entrySummary(entry: StreamEntry) {
  const at = iso(entry.at)
  switch (entry.kind) {
    case "event": {
      const { event } = entry
      return {
        at,
        kind: entry.kind,
        type: event.type,
        reason:
          event.type === "sync.reset" || event.type === "stream.closed"
            ? event.reason
            : undefined,
        id: entry.id,
        size: entry.size,
        effects: entry.effects,
        touched: entry.touched,
      }
    }
    case "connection":
      return { at, kind: entry.kind, connection: entry.connection }
    case "failure":
      return {
        at,
        kind: entry.kind,
        failure: entry.failure,
        action: entry.action,
      }
    case "wait":
      return { at, kind: entry.kind, ms: entry.ms }
    case "control":
      return { at, kind: entry.kind, control: entry.control }
  }
}

/** The recording and the cache as plain data, ready for `JSON.stringify`. */
export function diagnostics(
  queryClient: QueryClient,
  connections: Readonly<Record<string, Connection>>
) {
  const trace = traceNow()
  return {
    capturedAt: iso(Date.now()),
    recordingSince: iso(trace.since),
    page: window.location.pathname,
    userAgent: navigator.userAgent,
    clock: clockOffset(trace.clock),
    connections,
    streams: Object.fromEntries(
      Object.entries(trace.streams).map(([networkId, stream]) => [
        networkId,
        {
          cursor: stream.cursor,
          revision: stream.revision,
          synced: stream.synced,
          disconnectAfterMs: stream.disconnectAfterMs,
          events: stream.events,
          waits: stream.waits,
          entries: stream.entries.map(entrySummary),
        },
      ])
    ),
    requests: trace.requests.map((request) => ({
      ...request,
      at: iso(request.at),
    })),
    cache: queryClient
      .getQueryCache()
      .getAll()
      .map((query) => ({
        key: query.queryKey,
        status: query.state.status,
        fetchStatus: query.state.fetchStatus,
        stale: query.isStale(),
        updatedAt: query.state.dataUpdatedAt
          ? iso(query.state.dataUpdatedAt)
          : null,
        observers: query.getObserversCount(),
        recorded: trace.cache[query.queryHash] ?? null,
      })),
    attempts: unsettledAttempts(),
  }
}
