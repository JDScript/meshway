import { createFileRoute } from "@tanstack/react-router"

import { useNetworkSummary } from "@/features/network/queries"
import { NodesPage } from "@/features/nodes/page"
import { zNodesSearch } from "@/features/nodes/url"

/** The network's nodes, and one node or adding one over them (its child routes). */
export const Route = createFileRoute("/_authed/networks/$network/nodes")({
  staticData: { page: "nodes" },
  validateSearch: zNodesSearch,
  component: NodesRoute,
})

function NodesRoute() {
  const { network } = Route.useParams()
  return <NodesPage networkId={network} network={useNetworkSummary(network)} />
}
