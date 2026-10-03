import { listNetworkMembers } from "@meshway/control-client"
import { infiniteQueryOptions } from "@tanstack/react-query"

import { networkKey } from "@/features/network/queries"
import { authed, client } from "@/lib/api"

/** Members per page; the controller takes 1 to 200. */
const MEMBER_PAGE = 100

/**
 * The network's members, a page at a time. Only those who manage members
 * may read them; the controller answers others 403.
 */
export const membersQuery = (networkId: string) =>
  infiniteQueryOptions({
    queryKey: [...networkKey(networkId), "members"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listNetworkMembers(
          client,
          { networkId },
          pageParam === null
            ? { limit: MEMBER_PAGE }
            : { limit: MEMBER_PAGE, cursor: pageParam },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 30_000,
  })
