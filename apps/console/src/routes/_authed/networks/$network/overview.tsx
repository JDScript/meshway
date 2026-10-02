import { createFileRoute } from "@tanstack/react-router"
import { useCallback } from "react"
import { z } from "zod"

import { NODE_ID } from "@/features/network/queries"
import { NetworkOverview } from "@/features/overview/network-overview"

const zSearch = z.object({
  node: z.string().regex(NODE_ID).optional().catch(undefined),
})

export const Route = createFileRoute("/_authed/networks/$network/overview")({
  validateSearch: zSearch,
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
