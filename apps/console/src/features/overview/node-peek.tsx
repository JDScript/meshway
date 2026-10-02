import type { NodeSummary } from "@meshway/control-client"
import { ArrowUpRightIcon } from "lucide-react"
import { useState } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Button } from "@/components/ui/button"
import { Drawer, DrawerClose, DrawerContent, DrawerTitle } from "@/components/ui/drawer"
import { NodeAddresses, NodeLinks, NodeStatusTiles, nodeVitality, VitalityLine, type NodeContext } from "@/features/nodes/node-facts"
import { EDGE, MOBILE_EDGE, MOBILE_PEEK_HEIGHT, PANEL_TOP, PEEK_WIDTH } from "@/features/shell/layout"
import { m } from "@/paraglide/messages.js"

/**
 * The overview's quick look at one node, in shadcn's Drawer: from the right
 * on wide screens, from the bottom (swipeable) on small ones. Not modal: the
 * globe stays usable and picking another node swaps the content in place.
 * The full story is the node's workspace.
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
  // The last node stays on screen while the drawer slides away.
  const [kept, setKept] = useState(node)
  if (node !== null && node !== kept) setKept(node)
  const shown = node ?? kept

  return (
    <Drawer
      open={node !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      modal={false}
      disablePointerDismissal
      swipeDirection={mobile ? "down" : "right"}
      showSwipeHandle={mobile}
    >
      <DrawerContent
        initialFocus={false}
        finalFocus={false}
        className="glass rounded-sheet text-foreground data-ending-style:opacity-100"
        style={
          mobile
            ? { "--drawer-inset": `${MOBILE_EDGE}px`, "--bleed": "0px", height: `${MOBILE_PEEK_HEIGHT * 100}svh` }
            : { "--drawer-inset": `${EDGE}px`, "--drawer-content-width": `${PEEK_WIDTH}px`, "--bleed": "0px", marginTop: PANEL_TOP }
        }
      >
        {shown ? (
          <div key={shown.id} className="swap-in flex min-h-0 flex-1 flex-col">
            <SurfaceHeader
              className="px-4 pt-5 pb-4"
              title={(className) => <DrawerTitle className={className}>{shown.name}</DrawerTitle>}
              action={<DrawerClose render={<CloseButton />} />}
              meta={<VitalityLine vitality={nodeVitality(shown, context)} />}
            />
            <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-4 pb-4">
              <div className="flex flex-col gap-2">
                <NodeAddresses node={shown} />
                <div className="grid grid-cols-2 gap-2">
                  <NodeStatusTiles node={shown} context={context} />
                </div>
              </div>
              <NodeLinks node={shown} context={context} onOpenPeer={onPeek} />
            </div>
            <footer className="px-4 pb-4">
              <Button variant="secondary" className="h-10 w-full rounded-tile" onClick={() => onOpenWorkspace(shown.id)}>
                {m.node_open_workspace()}
                <ArrowUpRightIcon />
              </Button>
            </footer>
          </div>
        ) : null}
      </DrawerContent>
    </Drawer>
  )
}
