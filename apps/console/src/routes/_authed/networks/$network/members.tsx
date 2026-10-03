import { createFileRoute, redirect } from "@tanstack/react-router"

import { networkQuery } from "@/features/network/queries"

/** The members, for those who may manage them; `features/members/page.tsx` draws them. */
export const Route = createFileRoute("/_authed/networks/$network/members")({
  staticData: { page: "members" },
  loader: async ({ context: { queryClient }, params }) => {
    // The network's own route says when it cannot be read.
    const network = await queryClient
      .ensureQueryData(networkQuery(params.network))
      .catch(() => null)
    if (network && !network.network.actions.manageMembers)
      throw redirect({
        to: "/networks/$network/overview",
        params,
        replace: true,
      })
  },
})
