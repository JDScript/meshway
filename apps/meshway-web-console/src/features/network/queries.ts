import { getNetwork, listNetworks } from "@meshway/control-client"
import { infiniteQueryOptions, queryOptions } from "@tanstack/react-query"

import { ApiFailure, authed, client, problemCode } from "@/lib/api"

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
export const isNotFound = (error: unknown) =>
  error instanceof ApiFailure && problemCode(error.failure) === "not_found"
