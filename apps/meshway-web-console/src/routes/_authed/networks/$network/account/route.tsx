import { createFileRoute } from "@tanstack/react-router"

import { accountQuery } from "@/api/queries/account"
import { PageFrame } from "@/components/surface/page-frame"

import { useAccountNavigation } from "./-navigation"
import { AccountWorkspace } from "./-workspace"

/** This account and its sessions: a workspace with no panel. */
export const Route = createFileRoute("/_authed/networks/$network/account")({
  staticData: { page: "account", workspace: true },
  loader: ({ context }) => context.queryClient.prefetchQuery(accountQuery()),
  component: AccountPage,
})

/** The account page: one workspace, no panel. */
function AccountPage() {
  const { network: networkId } = Route.useParams()
  const nav = useAccountNavigation(networkId)
  return (
    <PageFrame
      networkId={networkId}
      page="account"
      workspace={<AccountWorkspace onClose={nav.close} />}
      onCloseWorkspace={nav.close}
    />
  )
}
