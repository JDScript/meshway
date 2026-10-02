import { createFileRoute, redirect } from "@tanstack/react-router"

import { isPolicyLayer, zPolicyWorkspaceSearch } from "@/features/policy/url"

/** One layer's workspace over the layers. An unknown layer is not one: back to the layers. */
export const Route = createFileRoute(
  "/_authed/networks/$network/policy/$layer"
)({
  staticData: { workspace: true },
  validateSearch: zPolicyWorkspaceSearch,
  beforeLoad: ({ params }) => {
    if (!isPolicyLayer(params.layer))
      throw redirect({
        to: "/networks/$network/policy",
        params: { network: params.network },
        replace: true,
      })
  },
})
