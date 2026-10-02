import { createFileRoute } from "@tanstack/react-router"

import { prefetchOverview } from "@/features/network/prefetch"
import { zPeekSearch } from "@/features/overview/url"

/** The globe on its own; `features/overview/page.tsx` draws the peek. */
export const Route = createFileRoute("/_authed/networks/$network/overview")({
  staticData: { page: "overview" },
  validateSearch: zPeekSearch,
  loader: ({ context, params }) =>
    prefetchOverview(context.queryClient, params.network),
  // What is open lives in the search; opening must not re-run the prefetch.
  shouldReload: false,
})
