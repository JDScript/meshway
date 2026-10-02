import { createFileRoute, redirect } from "@tanstack/react-router"

import { NODE_ID } from "@/features/network/ids"
import { zNodeWorkspaceSearch } from "@/features/nodes/url"

/** One node's workspace over the list. A malformed ID is not a node: back to the list. */
export const Route = createFileRoute("/_authed/networks/$network/nodes/$node")({
  staticData: { workspace: true },
  validateSearch: zNodeWorkspaceSearch,
  beforeLoad: ({ params }) => {
    if (!NODE_ID.test(params.node))
      throw redirect({
        to: "/networks/$network/nodes",
        params: { network: params.network },
        search: params.node === "tags" ? { view: "tags" } : {},
        replace: true,
      })
  },
})
