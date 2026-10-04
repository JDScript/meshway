import { createFileRoute, redirect } from "@tanstack/react-router"

import { networkQuery } from "@/api/queries/network"

/** Adding a node, for those who may edit the configuration; others go back to the list. */
export const Route = createFileRoute("/_authed/networks/$network/nodes/add")({
  staticData: { workspace: true },
  loader: async ({ context: { queryClient }, params }) => {
    // The network's own route says when it cannot be read.
    const network = await queryClient
      .ensureQueryData(networkQuery(params.network))
      .catch(() => null)
    if (network && !network.network.actions.editConfiguration)
      throw redirect({
        to: "/networks/$network/nodes",
        params,
        replace: true,
      })
  },
})
