import { zNodeId } from "@meshway/control-client/schemas"
import { createFileRoute, redirect } from "@tanstack/react-router"

import { zNodeSearch } from "@/features/nodes/url"

/** One node's workspace over the list. A malformed node ID is not one: back to the list. */
export const Route = createFileRoute("/_authed/networks/$network/nodes/$node")({
  staticData: { workspace: true },
  validateSearch: zNodeSearch,
  beforeLoad: ({ params, search }) => {
    if (!zNodeId.safeParse(params.node).success)
      throw redirect({
        to: "/networks/$network/nodes",
        params: { network: params.network },
        search: { q: search.q },
        replace: true,
      })
  },
})
