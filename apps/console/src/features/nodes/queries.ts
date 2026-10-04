import {
  getTransportCatalog,
  listEnrollmentKeys,
  listNodes,
} from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { isDenied } from "@/features/configuration/queries"
import { pollUnlessLive } from "@/features/live/status"
import { networkKey } from "@/features/network/queries"
import { authed, client } from "@/lib/api"

/**
 * The network's nodes as the controller lists them to this account: every
 * node for owners and admins, a member's own User-identity devices for
 * members. Registration and keys only; no runtime state. The event stream
 * writes it (its entities' nodes); it is polled when the stream is not live.
 */
export const nodesQuery = (networkId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "nodes"],
    queryFn: ({ signal }) =>
      authed((token) => listNodes(client, { networkId }, { token, signal })),
    staleTime: 10_000,
    refetchInterval: pollUnlessLive(networkId, 30_000),
  })

/** The network's enrolment keys, metadata only (a key's secret is never readable again). Owners and admins. */
export const enrollmentKeysQuery = (networkId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "enrollment-keys"],
    queryFn: ({ signal }) =>
      authed((token) =>
        listEnrollmentKeys(client, { networkId }, { token, signal })
      ),
    staleTime: 10_000,
    refetchInterval: (query) => (isDenied(query.state.error) ? false : 30_000),
  })

/** The transports this controller manages. */
export const transportCatalogQuery = () =>
  queryOptions({
    queryKey: ["transport-catalog"],
    queryFn: ({ signal }) =>
      authed((token) => getTransportCatalog(client, { token, signal })),
    staleTime: 5 * 60_000,
  })
