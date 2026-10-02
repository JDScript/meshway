import { createFileRoute } from "@tanstack/react-router"

import { prefetchOverview } from "@/features/network/prefetch"
import { zNodeListSearch } from "@/features/nodes/url"

/** The node list; its search and filter hold for every route below. `features/nodes/page.tsx` draws it. */
export const Route = createFileRoute("/_authed/networks/$network/nodes")({
  staticData: { page: "nodes" },
  validateSearch: zNodeListSearch,
  loader: ({ context, params }) =>
    prefetchOverview(context.queryClient, params.network),
  shouldReload: false,
})
