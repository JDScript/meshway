import { watchNetwork, type NetworkSummary } from "@meshway/control-client"
import type { QueryClient } from "@tanstack/react-query"

import { client } from "@/api/client"
import {
  configurationQuery,
  configurationReads,
  memberConfigurationQuery,
} from "@/api/queries/configuration"
import { configurationProgressQuery } from "@/api/queries/configuration-progress"
import { joinLinksQuery, membersQuery } from "@/api/queries/members"
import { networkKey, networkQuery, networksQuery } from "@/api/queries/network"
import { nodesQuery } from "@/api/queries/nodes"
import {
  observationListQuery,
  observationsKey,
  subscriptionQuery,
  writeRemoved,
  writeTask,
} from "@/api/queries/observation"
import { networkStateQuery } from "@/api/queries/state"
import { sameData } from "@/lib/same-data"
import { connectionOf, isLive, setConnection } from "@/state/live"
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
// reads poll (`pollUnlessLive`), and leaving live reads them again.

/** Marks the stream not live; the network's reads are read again and poll. */
function leaveLive(
  queryClient: QueryClient,
  networkId: string,
  dropped: boolean
) {
  const previous = connectionOf(networkId)
  if (previous?.phase !== "connecting" || (dropped && !previous.dropped))
    setConnection(networkId, {
      phase: "connecting",
      since: Date.now(),
      dropped,
    })
  if (previous?.phase === "live")
    void queryClient.invalidateQueries({ queryKey: networkKey(networkId) })
}

const ruleSetsKey = (networkId: string) =>
  [...networkKey(networkId), "rule-sets"] as const
const activityKey = (networkId: string) =>
  [...networkKey(networkId), "activity"] as const

/** The paged reads a membership change can alter. */
function membersChanged(queryClient: QueryClient, networkId: string) {
  void queryClient.invalidateQueries({
    queryKey: membersQuery(networkId).queryKey,
  })
  void queryClient.invalidateQueries({
    queryKey: joinLinksQuery(networkId).queryKey,
  })
  void queryClient.invalidateQueries({ queryKey: activityKey(networkId) })
}

/**
 * Writes `data` under `key` after cancelling a read of it in flight, which
 * would otherwise land later with what it read before.
 */
async function write<T>(
  queryClient: QueryClient,
  key: readonly unknown[],
  set: () => T
) {
  await queryClient.cancelQueries({ queryKey: key, exact: true })
  set()
}

async function apply(
  queryClient: QueryClient,
  networkId: string,
  effect: Effect
) {
  switch (effect.type) {
    case "install": {
      const { view, previous, progress } = effect
      await writeNetwork(queryClient, networkId, view.entities.network)
      const nodes = nodesQuery(networkId).queryKey
      await write(queryClient, nodes, () =>
        queryClient.setQueryData(nodes, inventoryOf(view.entities.nodes))
      )
      const state = networkStateQuery(networkId).queryKey
      await write(queryClient, state, () =>
        queryClient.setQueryData(state, view.state)
      )
      if (progress) {
        const key = configurationProgressQuery(
          networkId,
          progress.revision
        ).queryKey
        await write(queryClient, key, () =>
          queryClient.setQueryData(key, progress)
        )
      }
      const configuration = view.configuration
      if (configuration.type === "member") {
        const key = memberConfigurationQuery(networkId).queryKey
        queryClient.setQueryData(key, configuration)
      } else if (
        queryClient.getQueryData(configurationQuery(networkId).queryKey)
          ?.revision !== configuration.revision
      ) {
        void queryClient.invalidateQueries({
          queryKey: configurationReads(networkId),
        })
      }
      if (
        !previous ||
        !sameData(previous.entities.members, view.entities.members)
      )
        membersChanged(queryClient, networkId)
      // A new sync may follow missed observation events: read them again.
      void queryClient.invalidateQueries({
        queryKey: observationsKey(networkId),
      })
      return
    }
    case "entities": {
      const { previous, next } = effect
      if (!sameData(previous.network, next.network))
        await writeNetwork(queryClient, networkId, next.network)
      if (!sameData(previous.nodes, next.nodes)) {
        const key = nodesQuery(networkId).queryKey
        await write(queryClient, key, () =>
          queryClient.setQueryData(key, inventoryOf(next.nodes))
        )
      }
      if (!sameData(previous.members, next.members))
        membersChanged(queryClient, networkId)
      return
    }
    case "state": {
      const key = networkStateQuery(networkId).queryKey
      await write(queryClient, key, () =>
        queryClient.setQueryData(key, effect.state)
      )
      return
    }
    case "configuration":
      // The reads only: a draft keeps its text and base revision, and sees
      // the new revision through them.
      void queryClient.invalidateQueries({
        queryKey: configurationReads(networkId),
      })
      void queryClient.invalidateQueries({ queryKey: activityKey(networkId) })
      return
    case "progress": {
      const key = configurationProgressQuery(
        networkId,
        effect.progress.revision
      ).queryKey
      await write(queryClient, key, () =>
        queryClient.setQueryData(key, effect.progress)
      )
      return
    }
    case "resources":
      void queryClient.invalidateQueries({ queryKey: ruleSetsKey(networkId) })
      return
    case "observation":
    case "observation-removed": {
      const subscriptionId =
        effect.type === "observation"
          ? effect.task.subscriptionId
          : effect.subscriptionId
      if (!subscriptionId) return
      await queryClient.cancelQueries({
        queryKey: subscriptionQuery(queryClient, networkId, subscriptionId)
          .queryKey,
        exact: true,
      })
      await queryClient.cancelQueries({
        queryKey: observationListQuery(networkId).queryKey,
        exact: true,
      })
      if (effect.type === "observation")
        writeTask(queryClient, networkId, effect.task)
      else writeRemoved(queryClient, networkId, subscriptionId)
      return
    }
    case "unsynced":
    case "resync":
    case "closed":
      // The run loop acts on these: they change the connection.
      return
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
) {
  const key = networkQuery(networkId).queryKey
  const before = queryClient.getQueryData(key)
  if (!before || sameData(before.network, network)) return
  await write(queryClient, key, () =>
    queryClient.setQueryData(key, (old) => old && { ...old, network })
  )
  if (before.network.name !== network.name)
    void queryClient.invalidateQueries({ queryKey: networksQuery().queryKey })
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
  setConnection(networkId, {
    phase: "connecting",
    since: Date.now(),
    dropped: false,
  })

  // A mount undone at once (React's strict mode) never connects: the
  // controller holds each opened stream for a while and limits them per
  // session.
  await sleep(0, signal)
  while (!signal.aborted) {
    const session = currentSession()
    if (!session) return
    const connection = new AbortController()
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
        }
      )
      for await (const result of events) {
        if (signal.aborted) break
        if (!result.ok) {
          next = failureAction(result.error)
          break
        }
        const stepped = step(model, result.data.data, result.data.id)
        model = stepped.model
        let action: FailureAction | null = null
        for (const effect of stepped.effects) {
          await apply(queryClient, networkId, effect)
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
        if (action) {
          next = action
          break
        }
      }
    } finally {
      signal.removeEventListener("abort", abort)
      connection.abort()
    }
    if (signal.aborted) return
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
    leaveLive(queryClient, networkId, wasLive)
    await sleep(reconnectDelay(attempt++, Math.random()), signal)
  }
}

/** The stream is gone: the reads poll again, read afresh if it was live. */
export function stopNetworkStream(queryClient: QueryClient, networkId: string) {
  if (isLive(networkId))
    void queryClient.invalidateQueries({ queryKey: networkKey(networkId) })
  setConnection(networkId, null)
}
