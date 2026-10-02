import { createFileRoute } from "@tanstack/react-router"

import { policyQuery, tagsQuery } from "@/features/policy/queries"

/** The policy's layers; `features/policy/page.tsx` draws them. */
export const Route = createFileRoute("/_authed/networks/$network/policy")({
  staticData: { page: "policy" },
  loader: ({ context, params }) =>
    Promise.all([
      context.queryClient.prefetchQuery(policyQuery(params.network)),
      context.queryClient.prefetchQuery(tagsQuery(params.network)),
    ]),
})
