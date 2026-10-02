import {
  getEndpointDiscoveries,
  getManagedListeners,
  getTags,
  listAccountMembers,
  listNodeOwnership,
} from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { authed } from "@/lib/api"

// Reads of the nodes page. Configuration sources are read when a view
// needs them and after a change; observations poll only while they are on
// screen (a query's interval stops when nothing uses it). A failed refresh
// keeps the last data, which views label as such.

/** Saved listener intent and public material state; not proof of a bound or reachable listener. */
export const managedListenersQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "managed-listeners"],
    queryFn: () =>
      authed((base, token) => getManagedListeners(base, token, networkId)),
    staleTime: 30_000,
  })

/** Agents' own reports of what their actual UDP sockets observed through STUN. */
export const discoveriesQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "discoveries"],
    queryFn: () =>
      authed((base, token) => getEndpointDiscoveries(base, token, networkId)),
    refetchInterval: 12_000,
  })

/** The tag catalog and its active assignments, under the network's revision. */
export const tagsQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "tags"],
    queryFn: () => authed((base, token) => getTags(base, token, networkId)),
    staleTime: 30_000,
  })

export const ownershipQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "node-ownership"],
    queryFn: () =>
      authed((base, token) => listNodeOwnership(base, token, networkId)),
    staleTime: 30_000,
  })

export const membersQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "members"],
    queryFn: () =>
      authed((base, token) => listAccountMembers(base, token, networkId)),
    staleTime: 60_000,
  })
