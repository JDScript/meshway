import { createFileRoute } from "@tanstack/react-router"

/** Adding a node, over the list. */
export const Route = createFileRoute("/_authed/networks/$network/nodes/add")({
  staticData: { workspace: true },
})
