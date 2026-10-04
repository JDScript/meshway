import { createFileRoute } from "@tanstack/react-router"

import { OverviewPage } from "@/features/overview/page"

/** The globe on its own, with what it cannot show in a corner. */
export const Route = createFileRoute("/_authed/networks/$network/overview")({
  staticData: { page: "overview" },
  component: OverviewRoute,
})

function OverviewRoute() {
  const { network } = Route.useParams()
  return <OverviewPage networkId={network} />
}
