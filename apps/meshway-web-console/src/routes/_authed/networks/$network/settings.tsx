import { createFileRoute } from "@tanstack/react-router"

import { useNetworkSummary } from "@/features/network/queries"
import { SettingsPage } from "@/features/settings/page"

/** The settings sections, and one section over them (its child route). */
export const Route = createFileRoute("/_authed/networks/$network/settings")({
  staticData: { page: "settings" },
  component: SettingsRoute,
})

function SettingsRoute() {
  const { network } = Route.useParams()
  return (
    <SettingsPage networkId={network} network={useNetworkSummary(network)} />
  )
}
