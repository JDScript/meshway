import { create } from "zustand"

/**
 * What this tab's event stream for a network is doing, for the shell to
 * state: connecting (first or after a drop, `since` that moment), live
 * (`since` its sync was ready) or stopped for good (the account cannot
 * read the network). Times are the console's clock, in milliseconds.
 */
export type Connection =
  | { phase: "connecting"; since: number; dropped: boolean }
  | { phase: "live"; since: number }
  | { phase: "stopped"; since: number }

const useConnections = create<Record<string, Connection>>(() => ({}))

export function setConnection(
  networkId: string,
  connection: Connection | null
) {
  useConnections.setState((all) => {
    const next = { ...all }
    if (connection) next[networkId] = connection
    else delete next[networkId]
    return next
  }, true)
}

export function connectionOf(networkId: string): Connection | null {
  return useConnections.getState()[networkId] ?? null
}

export const useConnection = (networkId: string) =>
  useConnections((all) => all[networkId] ?? null)
