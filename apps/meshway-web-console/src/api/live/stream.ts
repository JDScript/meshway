import { watchNetwork, type NetworkSummary } from "@meshway/control-client"
import type { QueryClient, QueryKey } from "@tanstack/react-query"

import { client } from "@/api/client"
import {
  configurationQuery,
  configurationReads,
  memberConfigurationQuery,
} from "@/api/queries/configuration"
import { configurationProgressQuery } from "@/api/queries/configuration-progress"
import { joinLinksQuery, membersQuery } from "@/api/queries/members"
import {
  networkActivityQuery,
  networkKey,
  networkQuery,
  networksQuery,
} from "@/api/queries/network"
import { nodesQuery } from "@/api/queries/nodes"
import {
  observationListQuery,
  observationsKey,
  subscriptionQuery,
  writeRemoved,
  writeTask,
} from "@/api/queries/observation"
import { ruleSetsKey } from "@/api/queries/rule-sets"
import { networkStateQuery } from "@/api/queries/state"
import { sameData } from "@/lib/same-data"
import {
  streamTouches,
  traceStreamControl,
  traceStreamEvent,
  traceStreamFailure,
  traceStreamWait,
  type Touch,
} from "@/state/debug-trace"
import {
  connectionOf,
  isLive,
  setConnection,
  type Connection,
} from "@/state/live"
import { currentSession, rejectSession } from "@/state/session"

import {
  closedAction,
  failureAction,
  idleTimeout,
  initialModel,
  inventoryOf,
  reconnectDelay,
  step,
  type Effect,
  type FailureAction,
  type Model,
} from "./reducer"

// "SSE first, GET as the fallback": one `watchNetwork` stream per tab for
// the network it has open. Its snapshot and deltas write the reads they
// carry exactly (`reducer.ts`); paged lists and anything a GET adds
// (request IDs, cursors) are invalidated instead. While it is not live the
// reads poll (`pollUnlessLive`), and leaving live reads them again. Each
// change an event makes to the cache goes through `streamTouches`, which
// tells the debug mode which reads it touched.

/**
 * Sets the stream's connection; leaving live, the network's reads are read
 * again (and poll from then on).
 */
function changeConnection(
  queryClient: QueryClient,
  networkId: string,
  connection: Connection | null
) {
  const wasLive = isLive(networkId)
  setConnection(networkId, connection)
  if (wasLive && connection?.phase !== "live")
    void queryClient.invalidateQueries({ queryKey: networkKey(networkId) })
}

/** Marks the stream not live; the network's reads are read again and poll. */
function leaveLive(
  queryClient: QueryClient,
  networkId: string,
  dropped: boolean
) {
  const previous = connectionOf(networkId)
  if (previous?.phase !== "connecting" || (dropped && !previous.dropped))
    changeConnection(queryClient, networkId, {
      phase: "connecting",
      since: Date.now(),
      dropped,
    })
}

const invalidate = (queryClient: QueryClient, ...keys: QueryKey[]): Touch[] =>
  streamTouches(() => {
    for (const queryKey of keys)
      void queryClient.invalidateQueries({ queryKey })
  })

/** The paged reads a membership change can alter. */
const membersChanged = (queryClient: QueryClient, networkId: string) =>
  invalidate(
    queryClient,
    membersQuery(networkId).queryKey,
    joinLinksQuery(networkId).queryKey,
    networkActivityQuery(networkId).queryKey
  )

/**
 * Writes `data` under `key` after cancelling a read of it in flight, which
 * would otherwise land later with what it read before.
 */
async function write(
  queryClient: QueryClient,
  key: QueryKey,
  set: () => void
): Promise<Touch[]> {
  await queryClient.cancelQueries({ queryKey: key, exact: true })
  return streamTouches(set)
}

/** Applies one effect to the cache; returns the reads it touched. */
async function apply(
  queryClient: QueryClient,
  networkId: string,
  effect: Effect
): Promise<Touch[]> {
  switch (effect.type) {
    case "install": {
      const { view, previous, progress } = effect
      const touched = await writeNetwork(
        queryClient,
        networkId,
        view.entities.network
      )
      const nodes = nodesQuery(networkId).queryKey
      touched.push(
        ...(await write(queryClient, nodes, () =>
          queryClient.setQueryData(nodes, inventoryOf(view.entities.nodes))
        ))
      )
      const state = networkStateQuery(networkId).queryKey
      touched.push(
        ...(await write(queryClient, state, () =>
          queryClient.setQueryData(state, view.state)
        ))
      )
      if (progress) {
        const key = configurationProgressQuery(
          networkId,
          progress.revision
        ).queryKey
        touched.push(
          ...(await write(queryClient, key, () =>
            queryClient.setQueryData(key, progress)
          ))
        )
      }
      const configuration = view.configuration
      if (configuration.type === "member") {
        const key = memberConfigurationQuery(networkId).queryKey
        touched.push(
          ...streamTouches(() => queryClient.setQueryData(key, configuration))
        )
      } else if (
        queryClient.getQueryData(configurationQuery(networkId).queryKey)
          ?.revision !== configuration.revision
      ) {
        touched.push(...invalidate(queryClient, configurationReads(networkId)))
      }
      if (
        !previous ||
        !sameData(previous.entities.members, view.entities.members)
      )
        touched.push(...membersChanged(queryClient, networkId))
      // A new sync may follow missed observation events: read them again.
      touched.push(...invalidate(queryClient, observationsKey(networkId)))
      return touched
    }
    case "entities": {
      const { previous, next } = effect
      const touched: Touch[] = []
      if (!sameData(previous.network, next.network))
        touched.push(
          ...(await writeNetwork(queryClient, networkId, next.network))
        )
      if (!sameData(previous.nodes, next.nodes)) {
        const key = nodesQuery(networkId).queryKey
        touched.push(
          ...(await write(queryClient, key, () =>
            queryClient.setQueryData(key, inventoryOf(next.nodes))
          ))
        )
      }
      if (!sameData(previous.members, next.members))
        touched.push(...membersChanged(queryClient, networkId))
      return touched
    }
    case "state": {
      const key = networkStateQuery(networkId).queryKey
      return write(queryClient, key, () =>
        queryClient.setQueryData(key, effect.state)
      )
    }
    case "configuration":
      // The reads only: a draft keeps its text and base revision, and sees
      // the new revision through them.
      return invalidate(
        queryClient,
        configurationReads(networkId),
        networkActivityQuery(networkId).queryKey
      )
    case "progress": {
      const key = configurationProgressQuery(
        networkId,
        effect.progress.revision
      ).queryKey
      return write(queryClient, key, () =>
        queryClient.setQueryData(key, effect.progress)
      )
    }
    case "resources":
      return invalidate(queryClient, ruleSetsKey(networkId))
    case "observation":
    case "observation-removed": {
      const subscriptionId =
        effect.type === "observation"
          ? effect.task.subscriptionId
          : effect.subscriptionId
      if (!subscriptionId) return []
      await queryClient.cancelQueries({
        queryKey: subscriptionQuery(queryClient, networkId, subscriptionId)
          .queryKey,
        exact: true,
      })
      await queryClient.cancelQueries({
        queryKey: observationListQuery(networkId).queryKey,
        exact: true,
      })
      return streamTouches(() => {
        if (effect.type === "observation")
          writeTask(queryClient, networkId, effect.task)
        else writeRemoved(queryClient, networkId, subscriptionId)
      })
    }
    case "unsynced":
    case "resync":
    case "closed":
      // The run loop acts on these: they change the connection.
      return []
  }
}

/**
 * The summary as `getNetwork` returns it, inside the response read before
 * (whose request ID it keeps). Unread, it is left to the route's loader. A
 * new name also refreshes the network list.
 */
async function writeNetwork(
  queryClient: QueryClient,
  networkId: string,
  network: NetworkSummary
): Promise<Touch[]> {
  const key = networkQuery(networkId).queryKey
  const before = queryClient.getQueryData(key)
  if (!before || sameData(before.network, network)) return []
  const touched = await write(queryClient, key, () =>
    queryClient.setQueryData(key, (old) => old && { ...old, network })
  )
  if (before.network.name !== network.name)
    touched.push(...invalidate(queryClient, networksQuery().queryKey))
  return touched
}

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve) => {
    const timer = setTimeout(done, ms)
    signal.addEventListener("abort", done, { once: true })
    function done() {
      clearTimeout(timer)
      signal.removeEventListener("abort", done)
      resolve()
    }
  })

/** What the debug panel may do to a network's stream while it runs. */
export type StreamControls = {
  /** Drops the connection as a network failure would: it reconnects and resumes from its cursor. */
  drop: () => void
  /** Drops it and syncs afresh, without the cursor. */
  resync: () => void
  /** Drops it and stays away until `resume`; the reads poll meanwhile. */
  pause: () => void
  resume: () => void
}

/** The running stream of each network (runtime: functions, never state). */
const controls = new Map<string, StreamControls>()

export const streamControls = (networkId: string): StreamControls | null =>
  controls.get(networkId) ?? null

/**
 * Keeps `networkId`'s reads current until `signal` aborts: connects,
 * installs the sync, applies what follows, and on a drop reconnects with
 * bounded backoff, resuming from the last applied event. Ends for good when
 * the session is rejected (signing out, as every read does) or the account
 * cannot read the network (its reads are read again and say so). Once
 * `signal` aborts it changes no status: `stopNetworkStream` has.
 */
export async function runNetworkStream(
  queryClient: QueryClient,
  networkId: string,
  signal: AbortSignal
) {
  let model: Model = initialModel
  let attempt = 0
  let disconnectAfterMs: number | null = null
  /** A sync was ready at least once: a failure since is a drop. */
  let wasLive = false
  /** What the loop waits on now: a connection or the wait before one. */
  let waiting: AbortController | null = null
  /** What the debug panel asked, acted on once that wait ends. */
  let asked: "drop" | "resync" | "pause" | null = null
  let resumed: (() => void) | null = null
  const interrupt = (what: "drop" | "resync" | "pause") => {
    if (!waiting || asked) return
    asked = what
    traceStreamControl(networkId, what)
    waiting.abort()
  }
  const own: StreamControls = {
    drop: () => interrupt("drop"),
    resync: () => interrupt("resync"),
    pause: () => interrupt("pause"),
    resume: () => resumed?.(),
  }
  controls.set(networkId, own)
  setConnection(networkId, {
    phase: "connecting",
    since: Date.now(),
    dropped: false,
  })

  try {
    // A mount undone at once (React's strict mode) never connects: the
    // controller holds each opened stream for a while and limits them per
    // session.
    await sleep(0, signal)
    while (!signal.aborted) {
      if (asked === "pause") {
        changeConnection(queryClient, networkId, {
          phase: "paused",
          since: Date.now(),
        })
        await new Promise<void>((resolve) => {
          const done = () => {
            signal.removeEventListener("abort", done)
            resolve()
          }
          resumed = done
          signal.addEventListener("abort", done, { once: true })
        })
        resumed = null
        if (signal.aborted) return
        traceStreamControl(networkId, "resume")
        setConnection(networkId, {
          phase: "connecting",
          since: Date.now(),
          dropped: wasLive,
        })
        attempt = 0
      }
      if (asked === "resync") model = initialModel
      asked = null
      const session = currentSession()
      if (!session) return
      const connection = new AbortController()
      waiting = connection
      const abort = () => connection.abort()
      signal.addEventListener("abort", abort, { once: true })
      let next: FailureAction = "retry"
      try {
        const events = watchNetwork(
          client,
          { networkId },
          {
            token: session.token,
            signal: connection.signal,
            // Reconnecting is decided here, per connection.
            maxReconnects: 0,
            idleTimeoutMs: idleTimeout(disconnectAfterMs),
            resumeCursor: () => model.cursor ?? undefined,
            // A resumed stream sends no new sync: the controller taking the
            // cursor is what makes the installed view current again (what
            // was missed follows at once, or a reset if it cannot).
            onOpen: () => {
              if (!model.view) return
              attempt = 0
              setConnection(networkId, { phase: "live", since: Date.now() })
            },
          }
        )
        for await (const result of events) {
          if (signal.aborted || asked) break
          if (!result.ok) {
            next = failureAction(result.error)
            traceStreamFailure(networkId, result.error, next)
            break
          }
          const stepped = step(model, result.data.data, result.data.id)
          model = stepped.model
          let action: FailureAction | null = null
          const touched: Touch[] = []
          for (const effect of stepped.effects) {
            touched.push(...(await apply(queryClient, networkId, effect)))
            if (signal.aborted) break
            if (effect.type === "install") {
              attempt = 0
              wasLive = true
              disconnectAfterMs = effect.disconnectAfterMs
              setConnection(networkId, { phase: "live", since: Date.now() })
            } else if (effect.type === "unsynced")
              leaveLive(queryClient, networkId, false)
            else if (effect.type === "resync") action = "resync"
            else if (effect.type === "closed")
              action = closedAction(effect.reason)
          }
          traceStreamEvent(networkId, {
            id: result.data.id,
            event: result.data.data,
            effects: stepped.effects,
            touched,
            model,
          })
          if (action) {
            next = action
            break
          }
        }
      } finally {
        signal.removeEventListener("abort", abort)
        connection.abort()
        waiting = null
      }
      if (signal.aborted) return
      // The panel's interruption ended this connection, not a failure.
      if (asked) next = "retry"
      if (next === "stop") return stopNetworkStream(queryClient, networkId)
      if (next === "sign-out") {
        setConnection(networkId, { phase: "stopped", since: Date.now() })
        rejectSession(session)
        return
      }
      if (next === "lost") {
        // The network's reads say what the controller now answers.
        stopNetworkStream(queryClient, networkId)
        setConnection(networkId, { phase: "stopped", since: Date.now() })
        return
      }
      if (next === "resync") model = initialModel
      if (asked === "pause") continue
      leaveLive(queryClient, networkId, wasLive)
      const delay = reconnectDelay(attempt++, Math.random())
      traceStreamWait(networkId, delay)
      const wait = new AbortController()
      waiting = wait
      const stop = () => wait.abort()
      signal.addEventListener("abort", stop, { once: true })
      await sleep(delay, wait.signal)
      signal.removeEventListener("abort", stop)
      waiting = null
    }
  } finally {
    if (controls.get(networkId) === own) controls.delete(networkId)
  }
}

/** The stream is gone: the reads poll again, read afresh if it was live. */
export const stopNetworkStream = (
  queryClient: QueryClient,
  networkId: string
) => changeConnection(queryClient, networkId, null)
