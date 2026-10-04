import { createFileRoute } from "@tanstack/react-router"

import { zNodesSearch } from "@/features/nodes/url"

/** The network's nodes; `features/nodes/page.tsx` draws them. */
export const Route = createFileRoute("/_authed/networks/$network/nodes")({
  staticData: { page: "nodes" },
  validateSearch: zNodesSearch,
})
