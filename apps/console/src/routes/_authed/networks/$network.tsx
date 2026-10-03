import { zNetworkId } from "@meshway/control-client/schemas"
import { useQuery } from "@tanstack/react-query"
import { createFileRoute, notFound, Outlet } from "@tanstack/react-router"

import { sessionQuery } from "@/features/auth/queries"
import {
  NetworkFailed,
  NetworkNotFound,
} from "@/features/network/network-states"
import { rememberNetwork } from "@/features/network/preferences"
import { isNotFound, networkQuery } from "@/features/network/queries"
import { NetworkShell } from "@/features/shell/network-shell"
import { m } from "@/paraglide/messages.js"

/**
 * Every page under `/networks/<network_id>` is bound to that network. An
 * unknown or malformed ID, or one this account is not in, is a 404; it
 * never falls back to another network.
 */
export const Route = createFileRoute("/_authed/networks/$network")({
  loader: async ({ context: { queryClient }, params }) => {
    if (!zNetworkId.safeParse(params.network).success) throw notFound()
    const [session] = await Promise.all([
      queryClient.ensureQueryData(sessionQuery()),
      queryClient
        .ensureQueryData(networkQuery(params.network))
        .catch((error: unknown) => {
          if (isNotFound(error)) throw notFound()
          throw error
        }),
    ])
    rememberNetwork(session.user.id, params.network)
  },
  component: NetworkLayout,
  notFoundComponent: NetworkNotFound,
  errorComponent: ({ error }) => (
    <NetworkFailed title={m.network_failed()} error={error} />
  ),
})

/** One shell for every page of the network, so changing pages only moves panels. */
function NetworkLayout() {
  const { network: networkId } = Route.useParams()
  const network = useQuery(networkQuery(networkId))
  // Removed from the network, or the network is gone, since it was opened.
  if (isNotFound(network.error)) return <NetworkNotFound />
  return (
    <>
      <NetworkShell
        networkId={networkId}
        network={network.data?.network ?? null}
      />
      <Outlet />
    </>
  )
}
