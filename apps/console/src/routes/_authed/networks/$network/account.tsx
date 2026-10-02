import { createFileRoute } from "@tanstack/react-router"

import { networksQuery, sessionQuery } from "@/features/network/queries"

/** This account: its memberships, password and sign-ins. A workspace with no panel; `features/account/page.tsx` draws it. */
export const Route = createFileRoute("/_authed/networks/$network/account")({
  staticData: { page: "account", workspace: true },
  loader: ({ context }) =>
    Promise.allSettled([
      context.queryClient.prefetchQuery(sessionQuery()),
      context.queryClient.prefetchQuery(networksQuery()),
    ]),
})
