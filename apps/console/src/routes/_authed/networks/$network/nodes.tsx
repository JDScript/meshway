import { createFileRoute } from "@tanstack/react-router"

import { prefetchOverview } from "@/features/network/prefetch"
import { zNodeListSearch } from "@/features/shell/url"

/** The node list; its search and filter hold for every route below. The shell draws it. */
export const Route = createFileRoute("/_authed/networks/$network/nodes")({
  validateSearch: zNodeListSearch,
  loader: ({ context, params }) =>
    prefetchOverview(context.queryClient, params.network),
  shouldReload: false,
})
