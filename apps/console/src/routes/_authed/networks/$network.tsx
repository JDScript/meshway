import { createFileRoute, notFound, Outlet } from "@tanstack/react-router"
import { useEffect } from "react"

import { StatusCard } from "@/components/chrome/status-card"
import { DEFAULT_LOOK } from "@/features/globe/look"
import { setGlobe } from "@/features/globe/store"
import { EMPTY_SCENE } from "@/features/globe/scene"
import { NETWORK_ID, networksQuery, rememberNetwork } from "@/features/network/queries"
import { m } from "@/paraglide/messages.js"

/**
 * Every page under `/networks/<network_id>` is bound to that network. An unknown or
 * malformed ID is a 404; it never falls back to another network.
 */
export const Route = createFileRoute("/_authed/networks/$network")({
  loader: async ({ context, params }) => {
    if (!NETWORK_ID.test(params.network)) throw notFound()
    const { networks } = await context.queryClient.ensureQueryData(networksQuery())
    if (!networks.some((network) => network.network_id === params.network)) throw notFound()
    rememberNetwork(params.network)
  },
  component: Outlet,
  notFoundComponent: NetworkNotFound,
  errorComponent: NetworksFailed,
})

function useWelcomeGlobe() {
  useEffect(() => {
    setGlobe({ framing: "welcome", scene: EMPTY_SCENE, selectedNodeId: null, rightInset: 0, look: DEFAULT_LOOK })
  }, [])
}

function NetworkNotFound() {
  useWelcomeGlobe()
  return <StatusCard title={m.network_not_found()} />
}

function NetworksFailed({ reset }: { reset: () => void }) {
  useWelcomeGlobe()
  return <StatusCard title={m.networks_failed()} tone="destructive" onRetry={reset} />
}
