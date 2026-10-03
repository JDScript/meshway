import { createFileRoute } from "@tanstack/react-router"

import { accountQuery } from "@/features/account/queries"

/** This account and its sessions. A workspace with no panel; `features/account/page.tsx` draws it. */
export const Route = createFileRoute("/_authed/networks/$network/account")({
  staticData: { page: "account", workspace: true },
  loader: ({ context }) => context.queryClient.prefetchQuery(accountQuery()),
})
