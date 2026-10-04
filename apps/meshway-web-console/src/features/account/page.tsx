import { PageFrame } from "@/features/shell/page-frame"

import { AccountWorkspace } from "./account-workspace"
import { useAccountNavigation } from "./navigation"

/** The account page: one workspace, no panel. */
export function AccountPage({ networkId }: { networkId: string }) {
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
