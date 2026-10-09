import { createFileRoute, redirect } from "@tanstack/react-router"
import { z } from "zod"

import { MapPanels } from "../-map/panels"
import { useMapSelection, zMapSelection } from "../-map/selection"

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
        search: { node: search.node, group: search.group },
        replace: true,
      })
  },
  component: OverviewPage,
})

function OverviewPage() {
  const { network } = Route.useParams()
  const selection = useMapSelection(network, "overview")
  return <MapPanels networkId={network} page="overview" selection={selection} />
}
