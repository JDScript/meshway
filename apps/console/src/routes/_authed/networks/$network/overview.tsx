import { createFileRoute } from "@tanstack/react-router"

import { prefetchOverview } from "@/features/network/prefetch"
import { zPeekSearch } from "@/features/shell/url"

/** The globe on its own. The network shell draws every surface; see `$network.tsx`. */
export const Route = createFileRoute("/_authed/networks/$network/overview")({
  validateSearch: zPeekSearch,
  loader: ({ context, params }) =>
    prefetchOverview(context.queryClient, params.network),
  // What is open lives in the search; opening must not re-run the prefetch.
  shouldReload: false,
})
