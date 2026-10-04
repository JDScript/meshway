import { createFileRoute, redirect } from "@tanstack/react-router"

import { isSettingsSection } from "../-search"

/** One section's workspace over the sections. An unknown section is not one: back to the sections. */
export const Route = createFileRoute(
  "/_authed/networks/$network/settings/$section"
)({
  staticData: { workspace: true },
  beforeLoad: ({ params }) => {
    if (!isSettingsSection(params.section))
      throw redirect({
        to: "/networks/$network/settings",
        params: { network: params.network },
        replace: true,
      })
  },
})
