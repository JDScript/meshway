import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

/**
 * What this tab's event stream for a network is doing: connecting (first
 * or after a drop, `since` that moment), live (`since` its sync was ready)
 * or stopped for good (the account cannot read the network). Times are the
 * console's clock, in milliseconds. Written by the stream alone
 * (`api/live/stream.ts`); no entry while none runs.
 */
export type Connection =
  | { phase: "connecting"; since: number; dropped: boolean }
  | { phase: "live"; since: number }
  | { phase: "stopped"; since: number }

const liveStore = createStore<Readonly<Record<string, Connection>>>(() => ({}))

export function setConnection(
  networkId: string,
  connection: Connection | null
) {
  liveStore.setState((all) => {
    const next = { ...all }
    if (connection) next[networkId] = connection
    else delete next[networkId]
    return next
  }, true)
}

export const connectionOf = (networkId: string): Connection | null =>
  liveStore.getState()[networkId] ?? null

/**
 * Whether the stream keeps the network's cached reads current: from its
 * initial sync until it drops, resets or ends. Reads poll only while not.
 */
export const isLive = (networkId: string): boolean =>
  connectionOf(networkId)?.phase === "live"

export const useConnection = (networkId: string): Connection | null =>
  useStore(liveStore, (all) => all[networkId] ?? null)

export const useLive = (networkId: string): boolean =>
  useStore(liveStore, (all) => all[networkId]?.phase === "live")

/** A `refetchInterval` that polls every `ms` only while the network's stream is not live. */
export const pollUnlessLive = (networkId: string, ms: number) => () =>
  isLive(networkId) ? false : ms
