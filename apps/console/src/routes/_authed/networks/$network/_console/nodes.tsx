import { createFileRoute } from "@tanstack/react-router"

import { NodesPage } from "@/features/nodes/nodes-page"

export const Route = createFileRoute("/_authed/networks/$network/_console/nodes")({
  component: Nodes,
})

function Nodes() {
  const { network } = Route.useParams()
  return <NodesPage networkId={network} />
}
