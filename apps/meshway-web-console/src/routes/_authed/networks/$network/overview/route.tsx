import { createFileRoute, redirect } from "@tanstack/react-router"
import { z } from "zod"

import { MapPanel } from "../-map/map-panel"
import { zMapSelection } from "../-map/selection"

export const Route = createFileRoute("/_authed/networks/$network/overview")({
  staticData: { page: "overview" },
  validateSearch: zMapSelection.extend({
    view: z.literal("topology").optional().catch(undefined),
  }),
  beforeLoad: ({ params, search }) => {
    if (search.view === "topology")
      throw redirect({
        to: "/networks/$network/topology",
        params,
        search: { node: search.node },
        replace: true,
      })
  },
  component: OverviewPage,
})

function OverviewPage() {
  const { network } = Route.useParams()
  return <MapPanel networkId={network} page="overview" />
}
