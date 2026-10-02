import { PageFrame } from "@/features/shell/page-frame"

import { AccountWorkspace } from "./account-workspace"
import { useAccountNavigation, useAccountRoute } from "./navigation"

/** The account page: one workspace, no panel. */
export function AccountPage({ networkId }: { networkId: string }) {
  const route = useAccountRoute()
  const nav = useAccountNavigation(networkId)
  return (
    <PageFrame
      networkId={networkId}
      page="account"
      current={route.current}
      workspace={
        route.current ? (
          <AccountWorkspace networkId={networkId} onClose={nav.close} />
        ) : null
      }
      onCloseWorkspace={nav.close}
    />
  )
}
