import { zMembershipId } from "@meshway/control-client/schemas"
import { createFileRoute, redirect } from "@tanstack/react-router"

/** One member's workspace over the list. A malformed membership ID is not one: back to the list. */
export const Route = createFileRoute(
  "/_authed/networks/$network/members/$member"
)({
  staticData: { workspace: true },
  beforeLoad: ({ params, search }) => {
    if (!zMembershipId.safeParse(params.member).success)
      throw redirect({
        to: "/networks/$network/members",
        params: { network: params.network },
        search,
        replace: true,
      })
  },
})
