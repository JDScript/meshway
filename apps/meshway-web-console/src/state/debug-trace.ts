import type {
  ClientFailure,
  ErrorResponse,
  UserEvent,
} from "@meshway/control-client"
import type { QueryClient } from "@tanstack/react-query"
import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import type { Effect, FailureAction, Model } from "@/api/live/reducer"
import type { ClockSample } from "@/lib/debug/clock"

import { debugEnabled, onDebugToggle } from "./debug"
import { onConnectionChange, type Connection } from "./live"
import { onSessionChange } from "./session"

// What this tab did while the debug mode is on, for its panel: each
// network's stream (connections, events and what they wrote), the requests
// made, each cached read's history, and samples of the controller's clock.
// Memory only and bounded; it starts afresh when the mode turns on or off
// and when the sign-in changes. A credential, a header or a request's body
// is never recorded: events are what the cache holds anyway.

const STREAM_ENTRIES = 500
const REQUESTS = 500
const CLOCK_SAMPLES = 64

/** A cached read the stream wrote or marked stale while applying an event. */
export type Touch = { op: "write" | "invalidate"; key: readonly unknown[] }

/** What a failed connection or request was. */
export type FailureSummary = {
  kind: ClientFailure<ErrorResponse>["kind"]
  status: number | null
  code: string | null
}

/** What the debug panel did to a stream. */
export type StreamControl = "drop" | "resync" | "pause" | "resume"

type Stamp = { seq: number; at: number }

export type StreamEntry = Stamp &
  (
    | {
        kind: "event"
        id: string | null
        event: UserEvent
        /** The decoded event's JSON length, near its frame's size. */
        size: number
        effects: readonly Effect["type"][]
        touched: readonly Touch[]
      }
    | { kind: "connection"; connection: Connection | null }
    | { kind: "failure"; failure: FailureSummary; action: FailureAction }
    | { kind: "wait"; ms: number }
    | { kind: "control"; control: StreamControl }
  )

/** One network's stream as recorded. */
export type StreamTrace = {
  /** Oldest first, the last `STREAM_ENTRIES`. */
  entries: readonly StreamEntry[]
  /** After the last event: the cursor a reconnect resumes from, the view's revision, whether a sync is installed. */
  cursor: string | null
  revision: number | null
  synced: boolean
  /** The last sync's idle bound (`sync.ready`). */
  disconnectAfterMs: number | null
  events: number
  waits: number
  lastEventAt: number | null
}

export type RequestEntry = Stamp & {
  method: string
  /** The path and query only. */
  path: string
  /** The event stream's request: its time is to the response headers. */
  stream: boolean
  status: number | null
  /** To the response headers, in milliseconds. */
  ms: number
  code: string | null
  /** No response: the request failed or was cancelled. */
  failed: "network" | "cancelled" | null
}

/** One cached read's history while recording, by its query hash. */
export type CacheTrace = {
  fetches: number
  /** Data the stream put into the cache. */
  streamWrites: number
  /** Data anything else put into the cache without a fetch (a mutation's answer). */
  writes: number
  invalidations: number
  /** Where its data last came from. */
  lastUpdate: "fetch" | "stream" | "write" | null
}

type Trace = {
  /** When recording started. */
  since: number
  streams: Readonly<Record<string, StreamTrace>>
  requests: readonly RequestEntry[]
  cache: Readonly<Record<string, CacheTrace>>
  clock: readonly ClockSample[]
}

const empty = (): Trace => ({
  since: Date.now(),
  streams: {},
  requests: [],
  cache: {},
  clock: [],
})

const EMPTY_STREAM: StreamTrace = {
  entries: [],
  cursor: null,
  revision: null,
  synced: false,
  disconnectAfterMs: null,
  events: 0,
  waits: 0,
  lastEventAt: null,
}

const EMPTY_CACHE: CacheTrace = {
  fetches: 0,
  streamWrites: 0,
  writes: 0,
  invalidations: 0,
  lastUpdate: null,
}

const traceStore = createStore<Trace>(empty)

/** Collects what the cache reports while the stream changes it (`streamTouches`). */
let touching: Touch[] | null = null

let seq = 0
const stamp = (): Stamp => ({ seq: ++seq, at: Date.now() })

function appended<T>(list: readonly T[], item: T, max: number): T[] {
  const kept = list.length < max ? list : list.slice(list.length - max + 1)
  return [...kept, item]
}

function updateStream(
  networkId: string,
  change: (trace: StreamTrace) => StreamTrace
) {
  traceStore.setState((trace) => ({
    streams: {
      ...trace.streams,
      [networkId]: change(trace.streams[networkId] ?? EMPTY_STREAM),
    },
  }))
}

function addStreamEntry(networkId: string, entry: StreamEntry) {
  updateStream(networkId, (trace) => ({
    ...trace,
    entries: appended(trace.entries, entry, STREAM_ENTRIES),
  }))
}

function updateCache(hash: string, change: (trace: CacheTrace) => CacheTrace) {
  traceStore.setState((trace) => ({
    cache: { ...trace.cache, [hash]: change(trace.cache[hash] ?? EMPTY_CACHE) },
  }))
}

function summarize(failure: ClientFailure<ErrorResponse>): FailureSummary {
  return {
    kind: failure.kind,
    status: "status" in failure ? (failure.status ?? null) : null,
    code:
      failure.kind === "http" ? (failure.problem?.problem.code ?? null) : null,
  }
}

/** One event the stream applied: what it was, what it did, and the model after it. */
export function traceStreamEvent(
  networkId: string,
  applied: {
    id: string | undefined
    event: UserEvent
    effects: readonly Effect[]
    touched: readonly Touch[]
    model: Model
  }
) {
  if (!debugEnabled()) return
  const { id, event, effects, touched, model } = applied
  const at = stamp()
  updateStream(networkId, (trace) => ({
    entries: appended(
      trace.entries,
      {
        ...at,
        kind: "event",
        id: id ?? null,
        event,
        size: JSON.stringify(event).length,
        effects: effects.map((effect) => effect.type),
        touched,
      },
      STREAM_ENTRIES
    ),
    cursor: model.cursor,
    revision: model.view?.revision ?? null,
    synced: model.view !== null,
    disconnectAfterMs:
      event.type === "sync.ready"
        ? event.disconnectAfterMs
        : trace.disconnectAfterMs,
    events: trace.events + 1,
    waits: trace.waits,
    lastEventAt: at.at,
  }))
}

/**
 * Runs `change`, the stream's synchronous writes and invalidations of the
 * cache, and returns the reads the cache reports it touched; none while the
 * mode is off.
 */
export function streamTouches(change: () => void): Touch[] {
  if (!debugEnabled()) {
    change()
    return []
  }
  const outer = touching
  const touched: Touch[] = []
  touching = touched
  try {
    change()
  } finally {
    touching = outer
  }
  return touched
}

/** A connection that failed, and what the stream does about it. */
export function traceStreamFailure(
  networkId: string,
  failure: ClientFailure<ErrorResponse>,
  action: FailureAction
) {
  if (!debugEnabled()) return
  addStreamEntry(networkId, {
    ...stamp(),
    kind: "failure",
    failure: summarize(failure),
    action,
  })
}

/** The wait before the stream connects again. */
export function traceStreamWait(networkId: string, ms: number) {
  if (!debugEnabled()) return
  addStreamEntry(networkId, { ...stamp(), kind: "wait", ms })
  updateStream(networkId, (trace) => ({ ...trace, waits: trace.waits + 1 }))
}

export function traceStreamControl(networkId: string, control: StreamControl) {
  if (!debugEnabled()) return
  addStreamEntry(networkId, { ...stamp(), kind: "control", control })
}

/** One request, and the clock sample its `Date` header gave. */
export function traceRequest(
  request: Omit<RequestEntry, "seq" | "at">,
  at: number,
  clock: ClockSample | null
) {
  if (!debugEnabled()) return
  traceStore.setState((trace) => ({
    requests: appended(
      trace.requests,
      { ...request, seq: ++seq, at },
      REQUESTS
    ),
    clock: clock ? appended(trace.clock, clock, CLOCK_SAMPLES) : trace.clock,
  }))
}

/** Drops what was recorded; recording goes on. */
export const clearTrace = () => traceStore.setState(empty(), true)

/** What is recorded now, outside React (the diagnostics copy). */
export const traceNow = () => traceStore.getState()

/**
 * Records each cached read's fetches, writes and invalidations, and each
 * stream's connections, while the mode is on. Called once, by `main.tsx`.
 */
export function startDebugTrace(queryClient: QueryClient) {
  onSessionChange(clearTrace)
  onDebugToggle(clearTrace)
  onConnectionChange((networkId, connection) => {
    if (debugEnabled())
      addStreamEntry(networkId, { ...stamp(), kind: "connection", connection })
  })
  // The cache reports a write or an invalidation synchronously, inside the
  // call that made it: one made inside `streamTouches` is the stream's.
  queryClient.getQueryCache().subscribe((event) => {
    if (event.type !== "updated" || !debugEnabled()) return
    const { action, query } = event
    if (action.type === "invalidate") {
      touching?.push({ op: "invalidate", key: query.queryKey })
      updateCache(query.queryHash, (cache) => ({
        ...cache,
        invalidations: cache.invalidations + 1,
      }))
    } else if (action.type === "success") {
      const stream = action.manual === true && touching !== null
      if (stream) touching?.push({ op: "write", key: query.queryKey })
      updateCache(query.queryHash, (cache) =>
        !action.manual
          ? { ...cache, fetches: cache.fetches + 1, lastUpdate: "fetch" }
          : stream
            ? {
                ...cache,
                streamWrites: cache.streamWrites + 1,
                lastUpdate: "stream",
              }
            : { ...cache, writes: cache.writes + 1, lastUpdate: "write" }
      )
    }
  })
}

export const useTraceSince = (): number =>
  useStore(traceStore, (trace) => trace.since)

export const useStreamTrace = (networkId: string): StreamTrace =>
  useStore(traceStore, (trace) => trace.streams[networkId] ?? EMPTY_STREAM)

export const useRequestTrace = (): readonly RequestEntry[] =>
  useStore(traceStore, (trace) => trace.requests)

export const useCacheTrace = (): Readonly<Record<string, CacheTrace>> =>
  useStore(traceStore, (trace) => trace.cache)

export const useClockSamples = (): readonly ClockSample[] =>
  useStore(traceStore, (trace) => trace.clock)
