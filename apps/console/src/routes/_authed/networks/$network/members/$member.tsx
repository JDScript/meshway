import { createFileRoute, redirect } from "@tanstack/react-router"

import { USER_ID } from "@/features/members/url"

/** One member's workspace over the list. A malformed ID is not a member: back to the list. */
export const Route = createFileRoute(
  "/_authed/networks/$network/members/$member"
)({
  staticData: { workspace: true },
  beforeLoad: ({ params }) => {
    if (!USER_ID.test(params.member))
      throw redirect({
        to: "/networks/$network/members",
        params: { network: params.network },
        replace: true,
      })
  },
})
