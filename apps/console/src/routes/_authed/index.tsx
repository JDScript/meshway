import { createFileRoute, redirect } from "@tanstack/react-router"
import { useEffect } from "react"

import { StatusCard } from "@/components/chrome/status-card"
import { showWelcomeGlobe } from "@/features/globe/store"
import { networksQuery, rememberedNetwork } from "@/features/network/queries"
import { m } from "@/paraglide/messages.js"

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

function useWelcomeGlobe() {
  useEffect(() => {
    showWelcomeGlobe()
  }, [])
}

function NoNetwork() {
  useWelcomeGlobe()
  return (
    <StatusCard title={m.networks_none_title()} body={m.networks_none_body()} />
  )
}

function NetworksFailed({ reset }: { reset: () => void }) {
  useWelcomeGlobe()
  return (
    <StatusCard
      title={m.networks_failed()}
      tone="destructive"
      onRetry={reset}
    />
  )
}
