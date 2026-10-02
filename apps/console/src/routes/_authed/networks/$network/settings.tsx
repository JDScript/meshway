import { createFileRoute } from "@tanstack/react-router"

import { networkQuery } from "@/features/network/queries"

/** The settings sections; `features/settings/page.tsx` draws them. */
export const Route = createFileRoute("/_authed/networks/$network/settings")({
  staticData: { page: "settings" },
  loader: ({ context, params }) =>
    context.queryClient.prefetchQuery(networkQuery(params.network)),
})
