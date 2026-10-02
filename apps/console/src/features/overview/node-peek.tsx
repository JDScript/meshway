import type { NodeSummary } from "@meshway/control-client"
import { ArrowUpRightIcon } from "lucide-react"

import { CloseButton } from "@/components/chrome/close-button"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Button } from "@/components/ui/button"
import { DrawerClose, DrawerTitle } from "@/components/ui/drawer"
import { NodeAddresses, NodeLinks, NodeStatusTiles, nodeVitality, VitalityLine, type NodeContext } from "@/features/nodes/node-facts"
import { GlassDrawer } from "@/features/shell/glass-drawer"
import { EDGE, MOBILE_EDGE, MOBILE_PEEK_HEIGHT, PANEL_TOP, PEEK_WIDTH } from "@/features/shell/layout"
import { m } from "@/paraglide/messages.js"

/**
 * The overview's quick look at one node, in a glass drawer: from the right
 * on wide screens, from the bottom (swipeable) on small ones. The globe stays
 * usable and picking another node swaps the content in place. The full story
 * is the node's workspace.
 */
export function NodePeek({
  node,
  mobile,
  context,
  onPeek,
  onClose,
  onOpenWorkspace,
}: {
  /** null closes the drawer. */
  node: NodeSummary | null
  mobile: boolean
  context: NodeContext
  onPeek: (nodeId: string) => void
  onClose: () => void
  onOpenWorkspace: (nodeId: string) => void
}) {
  return (
    <GlassDrawer
      open={node !== null}
      onDismiss={onClose}
      side={mobile ? "bottom" : "right"}
      // On the globe, a tap elsewhere picks another node or empty space; the globe decides.
      dismissOnOutside={false}
      style={
        mobile
          ? { "--drawer-inset": `${MOBILE_EDGE}px`, height: `${MOBILE_PEEK_HEIGHT * 100}svh` }
          : { "--drawer-inset": `${EDGE}px`, "--drawer-content-width": `${PEEK_WIDTH}px`, marginTop: PANEL_TOP }
      }
    >
      {node ? (
        <div key={node.id} className="swap-in flex min-h-0 flex-1 flex-col">
          <SurfaceHeader
            className="px-4 pt-5 pb-4"
            title={(className) => <DrawerTitle className={className}>{node.name}</DrawerTitle>}
            action={<DrawerClose render={<CloseButton />} />}
            meta={<VitalityLine vitality={nodeVitality(node, context)} />}
          />
          <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-4 pb-4">
            <div className="flex flex-col gap-2">
              <NodeAddresses node={node} />
              <div className="grid grid-cols-2 gap-2">
                <NodeStatusTiles node={node} context={context} />
              </div>
            </div>
            <NodeLinks node={node} context={context} onOpenPeer={onPeek} />
          </div>
          <footer className="px-4 pb-4">
            <Button variant="secondary" className="h-10 w-full rounded-tile" onClick={() => onOpenWorkspace(node.id)}>
              {m.node_open_workspace()}
              <ArrowUpRightIcon />
            </Button>
          </footer>
        </div>
      ) : null}
    </GlassDrawer>
  )
}
