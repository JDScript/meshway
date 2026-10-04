import { createFileRoute } from "@tanstack/react-router"

import { useNetworkSummary } from "@/api/queries/network"

import { ConfigurationPage } from "./-configuration-page"

/** The configuration's sections, and one view over them (its child route). */
export const Route = createFileRoute(
  "/_authed/networks/$network/configuration"
)({
  staticData: { page: "configuration" },
  component: ConfigurationRoute,
})

function ConfigurationRoute() {
  const { network } = Route.useParams()
  return (
    <ConfigurationPage
      networkId={network}
      network={useNetworkSummary(network)}
    />
  )
}
