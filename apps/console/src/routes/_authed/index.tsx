import { createFileRoute } from "@tanstack/react-router"

import { AccountCard } from "@/features/account/account-card"
import { EntryPage } from "@/features/entry/frame"

/** `/`: the signed-in account. Network pages return with the controller's network operations. */
export const Route = createFileRoute("/_authed/")({
  component: () => (
    <EntryPage>
      <AccountCard />
    </EntryPage>
  ),
})
