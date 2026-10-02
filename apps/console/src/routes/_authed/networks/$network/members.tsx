import { createFileRoute } from "@tanstack/react-router"

import { zMembersSearch } from "@/features/members/url"
import { membersQuery, sessionQuery } from "@/features/network/queries"

/** The members, or the invitations; `features/members/page.tsx` draws them. */
export const Route = createFileRoute("/_authed/networks/$network/members")({
  staticData: { page: "members" },
  validateSearch: zMembersSearch,
  loader: ({ context, params }) =>
    Promise.allSettled([
      context.queryClient.prefetchQuery(sessionQuery()),
      context.queryClient.prefetchQuery(membersQuery(params.network)),
    ]),
  shouldReload: false,
})
