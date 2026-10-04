import { useSyncExternalStore } from "react"

/**
 * Whether this tab's event stream (`watchNetwork`) keeps a network's cached
 * reads current: `live` once its initial sync is ready, until it ends or
 * resets. Reads poll only while it is not.
 */
export type LiveStatus = "off" | "connecting" | "live"

const statuses = new Map<string, LiveStatus>()
const listeners = new Set<() => void>()

export const liveStatus = (networkId: string): LiveStatus =>
  statuses.get(networkId) ?? "off"

/**
 * Set by the stream alone. Leaving `live` must also refetch the network's
 * reads (invalidate `networkKey`), which re-evaluates their intervals.
 */
export function setLiveStatus(networkId: string, status: LiveStatus) {
  if (liveStatus(networkId) === status) return
  if (status === "off") statuses.delete(networkId)
  else statuses.set(networkId, status)
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export const useLiveStatus = (networkId: string) =>
  useSyncExternalStore(subscribe, () => liveStatus(networkId))

/** A `refetchInterval` that polls every `ms` only while the network's stream is not live. */
export const pollUnlessLive = (networkId: string, ms: number) => () =>
  liveStatus(networkId) === "live" ? false : ms
