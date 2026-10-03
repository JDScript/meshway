import { useNetwork } from "@/features/network/use-network-view"
import { useNodeNavigation } from "@/features/nodes/navigation"
import { useIsMobile } from "@/hooks/use-mobile"

import { FirstRun } from "./first-run"
import { usePeekedNode, usePeekNavigation } from "./navigation"
import { NodePeek } from "./node-peek"

/** The overview: the globe is its index, so its surfaces are the peek at one node and, with fewer than two nodes, what adding them takes. */
export function OverviewPage({ networkId }: { networkId: string }) {
  const view = useNetwork()
  const mobile = useIsMobile()
  const node = usePeekedNode(view.byId)
  const peek = usePeekNavigation(networkId)
  const nodes = useNodeNavigation(networkId)
  return (
    <>
      <FirstRun networkId={networkId} mobile={mobile} peek={node !== null} />
      <NodePeek
        node={node}
        mobile={mobile}
        context={view.context}
        onPeek={peek.peek}
        onClose={peek.closePeek}
        onOpenWorkspace={(nodeId) => nodes.openNode(nodeId)}
      />
    </>
  )
}
