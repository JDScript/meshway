import { createFileRoute } from "@tanstack/react-router"

/** The settings sections; `features/settings/page.tsx` draws them. */
export const Route = createFileRoute("/_authed/networks/$network/settings")({
  staticData: { page: "settings" },
})
