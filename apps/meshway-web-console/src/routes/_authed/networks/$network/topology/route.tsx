import { createFileRoute } from "@tanstack/react-router"

import { MapPanel } from "../-map/map-panel"
import { zMapSelection } from "../-map/selection"

/** The topology, drawn over the globe by the network's layout (`-map/topology-canvas.tsx`); this page holds its panel. */
export const Route = createFileRoute("/_authed/networks/$network/topology")({
  staticData: { page: "topology" },
  validateSearch: zMapSelection,
  component: TopologyPage,
})

function TopologyPage() {
  const { network } = Route.useParams()
  return <MapPanel networkId={network} page="topology" />
}
