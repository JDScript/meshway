import { createFileRoute, redirect } from "@tanstack/react-router"

import {
  isConfigurationView,
  zConfigurationSearch,
} from "@/features/configuration/url"

/** One view over the sections. An unknown view is not one: back to the sections. */
export const Route = createFileRoute(
  "/_authed/networks/$network/configuration/$view"
)({
  staticData: { workspace: true },
  validateSearch: zConfigurationSearch,
  beforeLoad: ({ params }) => {
    if (!isConfigurationView(params.view))
      throw redirect({
        to: "/networks/$network/configuration",
        params: { network: params.network },
        replace: true,
      })
  },
})
