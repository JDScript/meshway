import {
  getAccountSession,
  getConnectionRuntimes,
  getConnections,
  listNetworks,
  listNodes,
} from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { authed } from "@/lib/api"

// Reads keep their own cadence; a failed refresh keeps the last data, which
// views must label as such (TanStack Query keeps `data` with `error`).

export const sessionQuery = () =>
  queryOptions({
    queryKey: ["account", "session"],
    queryFn: () => authed(getAccountSession),
    staleTime: 60_000,
  })

export const networksQuery = () =>
  queryOptions({
    queryKey: ["networks"],
    queryFn: () => authed(listNetworks),
    staleTime: 30_000,
  })

export const nodesQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "nodes"],
    queryFn: () => authed((base, token) => listNodes(base, token, networkId)),
    refetchInterval: 15_000,
  })

export const connectionsQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "connections"],
    queryFn: () =>
      authed((base, token) => getConnections(base, token, networkId)),
    refetchInterval: 30_000,
  })

/** Agent self-reports; read separately so an outage never hides the saved plan. */
export const runtimesQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "runtimes"],
    queryFn: () =>
      authed((base, token) => getConnectionRuntimes(base, token, networkId)),
    refetchInterval: 12_000,
  })

const REMEMBERED_NETWORK = "meshway.network"

export function rememberedNetwork(): string | null {
  try {
    return window.localStorage.getItem(REMEMBERED_NETWORK)
  } catch {
    return null
  }
}

export function rememberNetwork(networkId: string) {
  try {
    window.localStorage.setItem(REMEMBERED_NETWORK, networkId)
  } catch {
    // A preference only.
  }
}

export const NETWORK_ID = /^[0-9a-f]{32}$/
export const NODE_ID = /^[0-9a-f]{64}$/
