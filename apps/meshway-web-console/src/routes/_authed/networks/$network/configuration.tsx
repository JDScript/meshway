import { createFileRoute } from "@tanstack/react-router"

/** The configuration's sections; `features/configuration/page.tsx` draws them. */
export const Route = createFileRoute(
  "/_authed/networks/$network/configuration"
)({
  staticData: { page: "configuration" },
})
