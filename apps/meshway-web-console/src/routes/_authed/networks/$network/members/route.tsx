import { createFileRoute, redirect } from "@tanstack/react-router"

import { networkQuery, useNetworkSummary } from "@/api/queries/network"

import { MembersPage } from "./-members-page"
import { zMembersSearch } from "./-search"

/** The members, for those who may manage them, and one member over them (its child route). */
export const Route = createFileRoute("/_authed/networks/$network/members")({
  staticData: { page: "members" },
  validateSearch: zMembersSearch,
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
  component: MembersRoute,
})

function MembersRoute() {
  const { network } = Route.useParams()
  return (
    <MembersPage networkId={network} network={useNetworkSummary(network)} />
  )
}
