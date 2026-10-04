import {
  getNetwork,
  listNetworkActivity,
  listNetworks,
  type NetworkSummary,
} from "@meshway/control-client"
import {
  infiniteQueryOptions,
  queryOptions,
  useQuery,
} from "@tanstack/react-query"

import { authed, client, codeOf } from "@/api/client"

// Network reads. Every key here belongs to this tab's sign-in: signing out,
// or the controller rejecting the session, clears the whole cache, and a
// session changed in another tab reloads this one.

/** Networks per page of the list; the controller takes 1 to 200. */
const NETWORK_PAGE = 50

/**
 * The networks this account belongs to, a page at a time. The cursor is
 * opaque and expires; refetching walks the pages again from the start, so
 * only "load more" can meet an expired one.
 */
export const networksQuery = () =>
  infiniteQueryOptions({
    queryKey: ["networks"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listNetworks(
          client,
          pageParam === null
            ? { limit: NETWORK_PAGE }
            : { limit: NETWORK_PAGE, cursor: pageParam },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 30_000,
  })

/** Every cached read of one network starts with this. */
export const networkKey = (networkId: string) => ["network", networkId] as const

/** One network as this account sees it: its name, this account's role and what it may do. */
export const networkQuery = (networkId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "summary"],
    queryFn: ({ signal }) =>
      authed((token) => getNetwork(client, { networkId }, { token, signal })),
    staleTime: 30_000,
  })

/** The controller says the network does not exist, or this account is not in it. */
export const isNotFound = (error: unknown) => codeOf(error) === "not_found"

/** The network as last read (its name, revision and this account's actions); null until read. */
export const useNetworkSummary = (networkId: string): NetworkSummary | null =>
  useQuery(networkQuery(networkId)).data?.network ?? null

/** Events per page; the controller takes 1 to 200. */
const ACTIVITY_PAGE = 50

/** What happened in the network, latest first; only those who manage members may read it. */
export const networkActivityQuery = (networkId: string) =>
  infiniteQueryOptions({
    queryKey: [...networkKey(networkId), "activity"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listNetworkActivity(
          client,
          { networkId },
          pageParam === null
            ? { limit: ACTIVITY_PAGE }
            : { limit: ACTIVITY_PAGE, cursor: pageParam },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 30_000,
  })
