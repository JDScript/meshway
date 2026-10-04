import {
  getObservation,
  listObservations,
  type ObservationTask,
} from "@meshway/control-client"
import { queryOptions, type QueryClient } from "@tanstack/react-query"

import { ApiFailure, authed, client, problemCode } from "@/api/client"
import {
  emptyRecord,
  foldTask,
  removeRecord,
  taskFor,
  type ObservationRecord,
} from "@/lib/observation/record"
import { pollUnlessLive } from "@/state/live"

import { networkKey } from "./network"

// Observation reads. The event stream writes them (`observation.status`,
// `observation.sample`, `observation.removed`); while it is not live they
// poll, and after a reset or a drop they are read again, which is how a
// subscription recovers what it missed: its current task, never the
// samples in between.

export const observationsKey = (networkId: string) =>
  [...networkKey(networkId), "observations"] as const

/** This sign-in's subscriptions in the network, as `listObservations` returns them. */
export const observationListQuery = (networkId: string) =>
  queryOptions({
    queryKey: [...observationsKey(networkId), "list"],
    queryFn: ({ signal }) =>
      authed((token) =>
        listObservations(client, { networkId }, { token, signal })
      ),
    staleTime: 5_000,
    refetchInterval: pollUnlessLive(networkId, 10_000),
  })

const isGone = (error: unknown) =>
  error instanceof ApiFailure && problemCode(error.failure) === "not_found"

/**
 * One subscription this tab follows, with the samples it has seen
 * (`record.ts`). A read folds the controller's current task into what is
 * kept; a subscription the controller no longer finds is removed.
 */
export const subscriptionQuery = (
  queryClient: QueryClient,
  networkId: string,
  subscriptionId: string
) => {
  const queryKey = [
    ...observationsKey(networkId),
    "subscription",
    subscriptionId,
  ] as const
  const options = queryOptions({
    queryKey,
    queryFn: async ({ signal }): Promise<ObservationRecord> => {
      const kept = queryClient.getQueryData(options.queryKey) ?? emptyRecord
      try {
        const list = await authed((token) =>
          getObservation(
            client,
            { networkId, subscriptionId },
            { token, signal }
          )
        )
        const task = taskFor(list.items, subscriptionId)
        return task ? foldTask(kept, task) : removeRecord(kept)
      } catch (error) {
        if (isGone(error)) return removeRecord(kept)
        throw error
      }
    },
    staleTime: Infinity,
    refetchInterval: pollUnlessLive(networkId, 2_000),
    retry: (count, error) => !isGone(error) && count < 2,
  })
  return options
}

/** Writes the controller's view of a subscription (a create, renew or event) into its record. */
export function writeTask(
  queryClient: QueryClient,
  networkId: string,
  task: ObservationTask
) {
  const subscriptionId = task.subscriptionId
  if (!subscriptionId) return
  queryClient.setQueryData(
    subscriptionQuery(queryClient, networkId, subscriptionId).queryKey,
    (old) => foldTask(old ?? emptyRecord, task)
  )
  queryClient.setQueryData(observationListQuery(networkId).queryKey, (old) => {
    if (!old) return old
    const index = old.items.findIndex(
      (item) => item.subscriptionId === subscriptionId
    )
    return {
      items: index === -1 ? [...old.items, task] : old.items.with(index, task),
    }
  })
}

/** A subscription removed: its record keeps what it measured, the list drops it. */
export function writeRemoved(
  queryClient: QueryClient,
  networkId: string,
  subscriptionId: string
) {
  queryClient.setQueryData(
    subscriptionQuery(queryClient, networkId, subscriptionId).queryKey,
    (old) => old && removeRecord(old)
  )
  queryClient.setQueryData(
    observationListQuery(networkId).queryKey,
    (old) =>
      old && {
        items: old.items.filter(
          (item) => item.subscriptionId !== subscriptionId
        ),
      }
  )
}
