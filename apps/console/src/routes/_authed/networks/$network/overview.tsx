import { createFileRoute } from "@tanstack/react-router"

/** The globe on its own; `features/overview/page.tsx` draws what is over it. */
export const Route = createFileRoute("/_authed/networks/$network/overview")({
  staticData: { page: "overview" },
})
