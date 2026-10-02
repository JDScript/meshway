import { Link } from "@tanstack/react-router"
import { PanelLeftIcon } from "lucide-react"
import { useEffect, useRef } from "react"
import { toast } from "sonner"

import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { buttonVariants } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { AccountMenu } from "@/features/auth/account-menu"
import { DEFAULT_LOOK } from "@/features/globe/look"
import { onGlobeSelect, setGlobe } from "@/features/globe/store"
import { useNetworkView } from "@/features/network/use-network-view"
import { useIsMobile } from "@/hooks/use-mobile"
import { m } from "@/paraglide/messages.js"

import { NetworkChip } from "./network-chip"
import { NodeCard } from "./node-card"

const CARD_WIDTH = 380
const EDGE = 24

/**
 * Globe mode: the network as nodes and links on the globe, the network and
 * its state top left, the selected node's card on the right.
 */
export function NetworkOverview({
  networkId,
  selectedNodeId,
  onSelect,
}: {
  networkId: string
  selectedNodeId: string | null
  onSelect: (nodeId: string | null) => void
}) {
  const view = useNetworkView(networkId)
  const mobile = useIsMobile()

  const selected = selectedNodeId ? view.byId.get(selectedNodeId) : undefined
  const shownSelection = selected && selected.revoked_at === null ? selected.id : null

  useEffect(() => {
    setGlobe({
      framing: "network",
      selectedNodeId: shownSelection,
      rightInset: shownSelection && !mobile ? CARD_WIDTH + EDGE * 2 : 0,
      look: DEFAULT_LOOK,
    })
  }, [shownSelection, mobile])

  useEffect(() => onGlobeSelect(onSelect), [onSelect])

  useEffect(() => {
    if (!shownSelection) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onSelect(null)
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [shownSelection, onSelect])

  // Read failures are announced once per outage; nothing changes size.
  useReadFailureToast("nodes", view.nodes.isError, m.overview_nodes_failed())

  return (
    <main className="ui-layer pointer-events-none fixed inset-0">
      <header className="absolute inset-x-0 top-0 flex items-center justify-between gap-3 p-4 md:p-5">
        <div className="enter">
          <NetworkChip networkId={networkId} name={view.network?.name ?? null} nodes={view.nodeCount} links={view.links} stale={view.stale} />
        </div>
        <div className="enter glass pointer-events-auto flex h-11 items-center gap-0.5 rounded-full px-1.5" style={{ "--enter-delay": "80ms" }}>
          <Tooltip>
            <TooltipTrigger
              render={
                <Link
                  to="/networks/$network/nodes"
                  params={{ network: networkId }}
                  aria-label={m.console_open()}
                  className={buttonVariants({ variant: "ghost", size: "icon" })}
                >
                  <PanelLeftIcon />
                </Link>
              }
            />
            <TooltipContent>{m.console_open()}</TooltipContent>
          </Tooltip>
          <LanguageSwitch />
          <ThemeSwitch />
          <AccountMenu />
        </div>
      </header>

      {shownSelection && selected ? (
        <aside
          key={selected.id}
          className="node-card-enter pointer-events-auto absolute inset-x-3 bottom-3 max-h-[64svh] md:inset-x-auto md:top-[84px] md:right-6 md:bottom-6 md:max-h-none md:w-[380px]"
        >
          <NodeCard
            node={selected}
            nodes={view.byId}
            observation={view.observation}
            runtimes={view.runtimes.data ?? null}
            now={view.now}
            onSelect={onSelect}
          />
        </aside>
      ) : null}
    </main>
  )
}

export function useReadFailureToast(id: string, failing: boolean, message: string) {
  const announced = useRef(false)
  useEffect(() => {
    if (failing && !announced.current) {
      announced.current = true
      toast.error(message, { id: `read-${id}` })
    }
    if (!failing) announced.current = false
  }, [failing, id, message])
}
