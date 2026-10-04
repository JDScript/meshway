import { createFileRoute } from "@tanstack/react-router"

import { AccountPage } from "@/features/account/page"
import { accountQuery } from "@/features/account/queries"

/** This account and its sessions: a workspace with no panel. */
export const Route = createFileRoute("/_authed/networks/$network/account")({
  staticData: { page: "account", workspace: true },
  loader: ({ context }) => context.queryClient.prefetchQuery(accountQuery()),
  component: AccountRoute,
})

function AccountRoute() {
  const { network } = Route.useParams()
  return <AccountPage networkId={network} />
}
