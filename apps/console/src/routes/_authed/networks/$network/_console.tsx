import { createFileRoute } from "@tanstack/react-router"

import { ConsoleLayout } from "@/features/console/console-layout"
import { prefetchOverview } from "@/features/network/prefetch"

/** Console mode: the sidebar with the docked globe, and a page beside it. */
export const Route = createFileRoute("/_authed/networks/$network/_console")({
  loader: ({ context, params }) => prefetchOverview(context.queryClient, params.network),
  component: ConsolePage,
})

function ConsolePage() {
  const { network } = Route.useParams()
  return <ConsoleLayout networkId={network} />
}
