import { createFileRoute, redirect } from "@tanstack/react-router"

import { NetworksFailed, NoNetwork } from "@/features/network/network-states"
import { rememberedNetwork } from "@/features/network/preferences"
import { networksQuery } from "@/features/network/queries"

/** `/`: the remembered network, else the first one this account may open. */
export const Route = createFileRoute("/_authed/")({
  loader: async ({ context }) => {
    const { networks } =
      await context.queryClient.ensureQueryData(networksQuery())
    const remembered = rememberedNetwork()
    const target =
      networks.find((network) => network.network_id === remembered) ??
      networks[0]
    if (target)
      throw redirect({
        to: "/networks/$network/overview",
        params: { network: target.network_id },
      })
  },
  component: NoNetwork,
  errorComponent: NetworksFailed,
})
