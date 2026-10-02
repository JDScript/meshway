import {
  getAccountSession,
  getConnectionRuntimes,
  getConnections,
  getNetwork,
  getTags,
  listAccountMembers,
  listNetworks,
  listNodeOwnership,
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

/** The network's own settings (name, DNS) and immutable pools, with their revision. */
export const networkQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "network"],
    queryFn: () => authed((base, token) => getNetwork(base, token, networkId)),
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

/** The tag catalog and its active assignments, under the network's revision. */
export const tagsQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "tags"],
    queryFn: () => authed((base, token) => getTags(base, token, networkId)),
    staleTime: 30_000,
  })

/** Live memberships; every member may read them. */
export const membersQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "members"],
    queryFn: () =>
      authed((base, token) => listAccountMembers(base, token, networkId)),
    staleTime: 30_000,
  })

/** Which membership owns each node; null is a node no membership owns yet. */
export const ownershipQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "ownership"],
    queryFn: () =>
      authed((base, token) => listNodeOwnership(base, token, networkId)),
    staleTime: 30_000,
  })
