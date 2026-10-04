import { zNetworkId } from "@meshway/control-client/schemas"
import { useQuery } from "@tanstack/react-query"
import { createFileRoute, notFound, Outlet } from "@tanstack/react-router"

import { useNetworkStream } from "@/api/live/use-network-stream"
import { sessionQuery } from "@/api/queries/auth"
import { isNotFound, networkQuery } from "@/api/queries/network"
import {
  NetworkFailed,
  NetworkNotFound,
} from "@/components/entry/network-states"
import { rememberNetwork } from "@/lib/remembered-network-storage"
import { m } from "@/paraglide/messages.js"

import { NetworkShell } from "./-shell/network-shell"

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

/** One shell and one event stream for every page of the network; each page's route draws its own surfaces. */
function NetworkLayout() {
  const { network: networkId } = Route.useParams()
  useNetworkStream(networkId)
  const network = useQuery(networkQuery(networkId))
  // Removed from the network, or the network is gone, since it was opened.
  if (isNotFound(network.error)) return <NetworkNotFound />
  // The page's surfaces first: the chrome sits above them.
  return (
    <>
      <Outlet />
      <NetworkShell
        networkId={networkId}
        network={network.data?.network ?? null}
      />
    </>
  )
}
