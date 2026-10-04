import { cancelObservation, renewObservation } from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect, useState } from "react"

import { ApiFailure, authed, client, problemCode } from "@/api/client"
import {
  subscriptionQuery,
  writeRemoved,
  writeTask,
} from "@/api/queries/observation"
import { useDocumentVisible } from "@/hooks/use-document-visible"
import { useNow } from "@/hooks/use-now"
import { leasePlan, retryPlan } from "@/lib/observation/lease"
import { standing, taskFor } from "@/lib/observation/record"

/** Cancellations due once a holder has unmounted, unless it mounts again at once. */
const leaving = new Map<string, number>()

/**
 * One subscription this tab holds, while it is mounted: its record (kept
 * current by the event stream, read again when it is not live), renewed at
 * the controller's times while the tab is visible, read again once its
 * lease runs out, and cancelled when it unmounts.
 */
export function useSubscription(networkId: string, subscriptionId: string) {
  const queryClient = useQueryClient()
  const query = useQuery(
    subscriptionQuery(queryClient, networkId, subscriptionId)
  )
  const visible = useDocumentVisible()
  const now = useNow(1_000)
  const record = query.data ?? null
  const task = record?.task ?? null
  const current = record
    ? standing(record, now)
    : ({ type: "unknown" } as const)
  const [renewFailure, setRenewFailure] = useState<unknown>(null)
  /** Counts renewals refused as early, to try again (`retryPlan`). */
  const [early, setEarly] = useState(0)

  const phase = task?.phase ?? null
  const renewAfter = task?.renewAfter ?? null
  const subscriberExpiresAt = task?.subscriberExpiresAt ?? null
  const expiresAt = task?.expiresAt ?? null
  const removed = record?.removed ?? false

  useEffect(() => {
    if (!visible || removed || !phase || !renewAfter || !expiresAt) return
    const times = { phase, renewAfter, subscriberExpiresAt, expiresAt }
    const plan = early
      ? retryPlan(times, Date.now())
      : leasePlan(times, Date.now())
    if (plan.type !== "renew") return
    let live = true
    const timer = window.setTimeout(() => {
      void authed((token) =>
        renewObservation(client, { networkId, subscriptionId }, { token })
      ).then(
        (list) => {
          if (!live) return
          const next = taskFor(list.items, subscriptionId)
          if (next) writeTask(queryClient, networkId, next)
          else writeRemoved(queryClient, networkId, subscriptionId)
          setRenewFailure(null)
          setEarly(0)
        },
        (error: unknown) => {
          if (!live) return
          setRenewFailure(error)
          const code =
            error instanceof ApiFailure ? problemCode(error.failure) : null
          if (code === "observation_limit") setEarly((n) => n + 1)
          else if (code === "not_found")
            writeRemoved(queryClient, networkId, subscriptionId)
        }
      )
    }, plan.inMs)
    return () => {
      live = false
      window.clearTimeout(timer)
    }
  }, [
    queryClient,
    networkId,
    subscriptionId,
    visible,
    removed,
    phase,
    renewAfter,
    subscriberExpiresAt,
    expiresAt,
    early,
  ])

  // A lease that ran out: ask the controller what it now says.
  const lapsed = current.type === "lapsed"
  const { refetch } = query
  useEffect(() => {
    if (lapsed) void refetch()
  }, [lapsed, refetch])

  // Leaving (closing the panel, another tab of the node, another network)
  // cancels the subscription; for one that has ended it changes nothing. A
  // mount undone and redone at once (React's strict mode) keeps it.
  useEffect(() => {
    const key = `${networkId}/${subscriptionId}`
    window.clearTimeout(leaving.get(key))
    leaving.delete(key)
    return () => {
      leaving.set(
        key,
        window.setTimeout(() => {
          leaving.delete(key)
          void authed((token) =>
            cancelObservation(client, { networkId, subscriptionId }, { token })
          ).catch(() => undefined)
        }, 0)
      )
    }
  }, [networkId, subscriptionId])

  return { query, record, standing: current, visible, renewFailure, now }
}
