import { getAuthOptions, getAuthSession } from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { authed, call, client } from "@/lib/api"

/** Whether this controller accepts registrations, and its password rules. */
export const authOptionsQuery = () =>
  queryOptions({
    queryKey: ["auth", "options"],
    queryFn: ({ signal }) => call(getAuthOptions(client, { signal })),
    staleTime: 60_000,
  })

/** This tab's session as the controller sees it: the account and the session's own times. */
export const sessionQuery = () =>
  queryOptions({
    queryKey: ["account", "session"],
    queryFn: ({ signal }) =>
      authed((token) => getAuthSession(client, { token, signal })),
    staleTime: 60_000,
  })
