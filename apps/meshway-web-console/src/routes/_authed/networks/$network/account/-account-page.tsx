import { PageFrame } from "@/components/surface/page-frame"

import { useAccountNavigation } from "./-navigation"
import { AccountWorkspace } from "./-workspace"

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
