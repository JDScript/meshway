import {
  getAccount,
  listAccountActivity,
  listAccountSessions,
} from "@meshway/control-client"
import { infiniteQueryOptions, queryOptions } from "@tanstack/react-query"

import { authed, client } from "@/api/client"

/** Rows per page; the controller takes 1 to 200. */
const PAGE = 50

const page = (cursor: string | null) =>
  cursor === null ? { limit: PAGE } : { limit: PAGE, cursor }

/** This account as the controller reports it: its ID and email. */
export const accountQuery = () =>
  queryOptions({
    queryKey: ["account", "user"],
    queryFn: ({ signal }) =>
      authed((token) => getAccount(client, { token, signal })),
    staleTime: 60_000,
  })

/** The account's sessions that still hold, newest first, a page at a time. */
export const accountSessionsQuery = () =>
  infiniteQueryOptions({
    queryKey: ["account", "sessions"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listAccountSessions(client, page(pageParam), { token, signal })
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 15_000,
  })

/** What happened to the account, newest first, a page at a time. */
export const accountActivityQuery = () =>
  infiniteQueryOptions({
    queryKey: ["account", "activity"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listAccountActivity(client, page(pageParam), { token, signal })
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 15_000,
  })
