import { createFileRoute, redirect } from "@tanstack/react-router"

import { NODE_ID } from "@/features/network/queries"
import { zNodeWorkspaceSearch } from "@/features/shell/url"

/** One node's workspace over the list. A malformed ID is not a node: back to the list. */
export const Route = createFileRoute("/_authed/networks/$network/nodes/$node")({
  validateSearch: zNodeWorkspaceSearch,
  beforeLoad: ({ params }) => {
    if (!NODE_ID.test(params.node))
      throw redirect({
        to: "/networks/$network/nodes",
        params: { network: params.network },
        replace: true,
      })
  },
})
