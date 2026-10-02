import { createFileRoute } from "@tanstack/react-router"
import { useCallback } from "react"
import { z } from "zod"

import { prefetchOverview } from "@/features/network/prefetch"
import { NODE_ID } from "@/features/network/queries"
import { NetworkOverview } from "@/features/overview/network-overview"

const zSearch = z.object({
  node: z.string().regex(NODE_ID).optional().catch(undefined),
})

export const Route = createFileRoute("/_authed/networks/$network/overview")({
  validateSearch: zSearch,
  loader: ({ context, params }) => prefetchOverview(context.queryClient, params.network),
  // Selection lives in the search params; picking a node must not re-run the prefetch.
  shouldReload: false,
  component: OverviewPage,
})

function OverviewPage() {
  const { network } = Route.useParams()
  const { node } = Route.useSearch()
  const navigate = Route.useNavigate()
  const select = useCallback(
    (next: string | null) => void navigate({ search: { node: next ?? undefined }, replace: true }),
    [navigate]
  )
  return <NetworkOverview networkId={network} selectedNodeId={node ?? null} onSelect={select} />
}
