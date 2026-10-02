import {
  getEndpointDiscoveries,
  getManagedListeners,
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
