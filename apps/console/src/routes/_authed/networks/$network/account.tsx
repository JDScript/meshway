import { createFileRoute } from "@tanstack/react-router"

import { sessionQuery } from "@/features/auth/queries"

/** This account and this sign-in. A workspace with no panel; `features/account/page.tsx` draws it. */
export const Route = createFileRoute("/_authed/networks/$network/account")({
  staticData: { page: "account", workspace: true },
  loader: ({ context }) => context.queryClient.prefetchQuery(sessionQuery()),
})
