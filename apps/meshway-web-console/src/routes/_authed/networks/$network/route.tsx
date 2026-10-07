import { zNetworkId } from "@meshway/control-client/schemas"
import { useQuery } from "@tanstack/react-query"
import { createFileRoute, notFound, Outlet } from "@tanstack/react-router"

import { useNetworkStream } from "@/api/live/use-network-stream"
import { sessionQuery } from "@/api/queries/auth"
import { isNotFound, networkQuery } from "@/api/queries/network"
import { PageNotFound } from "@/components/entry/fallback"
import {
  NetworkFailed,
  NetworkNotFound,
} from "@/components/entry/network-states"
import { rememberNetwork } from "@/lib/remembered-network"
import { m } from "@/paraglide/messages.js"

import { NetworkShell } from "./-shell/network-shell"

/** What the loader's not-found carries: the network itself is missing (not a page under it). */
const NO_NETWORK = "network"

/**
 * Every page under `/networks/<network_id>` is bound to that network. An
 * unknown or malformed ID, or one this account is not in, is a 404; it
 * never falls back to another network. An address under a network with no
 * page is that network's page not found, in its shell.
 */
export const Route = createFileRoute("/_authed/networks/$network")({
  loader: async ({ context: { queryClient }, params }) => {
    if (!zNetworkId.safeParse(params.network).success)
      throw notFound({ data: NO_NETWORK })
    const [session] = await Promise.all([
      queryClient.ensureQueryData(sessionQuery()),
      queryClient
        .ensureQueryData(networkQuery(params.network))
        .catch((error: unknown) => {
          if (isNotFound(error)) throw notFound({ data: NO_NETWORK })
          throw error
        }),
    ])
    rememberNetwork(session.user.id, params.network)
  },
  component: NetworkLayout,
  notFoundComponent: ({ data }) =>
    data === NO_NETWORK ? <NetworkNotFound /> : <PageNotFound />,
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
