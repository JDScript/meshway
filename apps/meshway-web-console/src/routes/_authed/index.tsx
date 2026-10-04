import { createFileRoute, redirect } from "@tanstack/react-router"

import { SessionRejected } from "@/api/client"
import { sessionQuery } from "@/api/queries/auth"
import { networkQuery, networksQuery } from "@/api/queries/network"
import { EntryPage } from "@/components/entry/frame"
import { FirstNetwork, NetworkFailed } from "@/components/entry/network-states"
import { landing } from "@/lib/remembered-network"
import { rememberedNetwork } from "@/lib/remembered-network-storage"
import { m } from "@/paraglide/messages.js"

/** `/`: the remembered network, else the first one this account may open, else creating one. */
export const Route = createFileRoute("/_authed/")({
  loader: async ({ context: { queryClient } }) => {
    const [session, networks] = await Promise.all([
      queryClient.ensureQueryData(sessionQuery()),
      queryClient.ensureInfiniteQueryData(networksQuery()),
    ])
    const first = networks.pages[0]
    const { open, check } = landing(
      first?.items ?? [],
      (first?.nextCursor ?? null) !== null,
      rememberedNetwork(session.user.id)
    )
    const target =
      check !== null &&
      (await queryClient.ensureQueryData(networkQuery(check)).then(
        () => true,
        (error: unknown) => {
          if (error instanceof SessionRejected) throw error
          return false
        }
      ))
        ? check
        : open
    if (target !== null)
      throw redirect({
        to: "/networks/$network/overview",
        params: { network: target },
        replace: true,
      })
  },
  component: () => (
    <EntryPage>
      <FirstNetwork />
    </EntryPage>
  ),
  errorComponent: ({ error }) => (
    <NetworkFailed title={m.networks_failed()} error={error} />
  ),
})
