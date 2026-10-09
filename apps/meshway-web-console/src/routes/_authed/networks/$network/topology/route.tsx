import { createFileRoute } from "@tanstack/react-router"

import { MapPanels } from "../-map/panels"
import { useMapSelection, zMapSelection } from "../-map/selection"

export const Route = createFileRoute("/_authed/networks/$network/topology")({
  staticData: { page: "topology" },
  validateSearch: zMapSelection,
  component: TopologyPage,
})

function TopologyPage() {
  const { network } = Route.useParams()
  const selection = useMapSelection(network, "topology")
  return <MapPanels networkId={network} page="topology" selection={selection} />
}
