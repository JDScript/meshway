import { createFileRoute, notFound, Outlet } from "@tanstack/react-router"

import { NETWORK_ID } from "@/features/network/ids"
import {
  NetworkNotFound,
  NetworksFailed,
} from "@/features/network/network-states"
import { rememberNetwork } from "@/features/network/preferences"
import { networksQuery } from "@/features/network/queries"
import { NetworkShell } from "@/features/shell/network-shell"

/**
 * Every page under `/networks/<network_id>` is bound to that network. An
 * unknown or malformed ID is a 404; it never falls back to another network.
 */
export const Route = createFileRoute("/_authed/networks/$network")({
  loader: async ({ context, params }) => {
    if (!NETWORK_ID.test(params.network)) throw notFound()
    const { networks } =
      await context.queryClient.ensureQueryData(networksQuery())
    if (!networks.some((network) => network.network_id === params.network))
      throw notFound()
    rememberNetwork(params.network)
  },
  component: NetworkLayout,
  notFoundComponent: NetworkNotFound,
  errorComponent: NetworksFailed,
})

/** One shell for every page of the network, so changing pages only moves panels. */
function NetworkLayout() {
  const { network } = Route.useParams()
  return (
    <>
      <NetworkShell networkId={network} />
      <Outlet />
    </>
  )
}
