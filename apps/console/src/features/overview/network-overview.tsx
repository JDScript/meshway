import { useQuery } from "@tanstack/react-query"
import { useEffect, useMemo, useRef } from "react"
import { toast } from "sonner"

import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { AccountMenu } from "@/features/auth/account-menu"
import { DEFAULT_LOOK } from "@/features/globe/look"
import { onGlobeSelect, setGlobe } from "@/features/globe/store"
import { connectionsQuery, networksQuery, nodesQuery, runtimesQuery } from "@/features/network/queries"
import { useIsMobile } from "@/hooks/use-mobile"
import { clockTime } from "@/lib/format"
import { useNow } from "@/lib/use-now"
import { m } from "@/paraglide/messages.js"

import { linkSummary, overviewScene } from "./model"
import { NodeCard } from "./node-card"
import { NetworkChip } from "./network-chip"

const CARD_WIDTH = 380
const EDGE = 24

/**
 * Globe mode: the network as nodes and links on the globe, the overall
 * state in a corner and the selected node's card on the right.
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
  const networks = useQuery(networksQuery())
  const nodes = useQuery(nodesQuery(networkId))
  const connections = useQuery(connectionsQuery(networkId))
  const runtimes = useQuery(runtimesQuery(networkId))
  const now = useNow()
  const mobile = useIsMobile()

  const network = networks.data?.networks.find((candidate) => candidate.network_id === networkId)
  const inventory = useMemo(() => nodes.data?.nodes ?? [], [nodes.data])
  const byId = useMemo(() => new Map(inventory.map((node) => [node.id, node])), [inventory])
  const { scene, observation } = useMemo(
    () => overviewScene({ nodes: inventory, connections: connections.data ?? null, runtimes: runtimes.data ?? null, now }),
    [inventory, connections.data, runtimes.data, now]
  )
  const selected = selectedNodeId ? byId.get(selectedNodeId) : undefined
  const shownSelection = selected && selected.revoked_at === null ? selected.id : null

  useEffect(() => {
    setGlobe({
      framing: "network",
      scene,
      selectedNodeId: shownSelection,
      rightInset: shownSelection && !mobile ? CARD_WIDTH + EDGE * 2 : 0,
      look: DEFAULT_LOOK,
    })
  }, [scene, shownSelection, mobile])

  useEffect(() => onGlobeSelect(onSelect), [onSelect])

  useEffect(() => {
    if (!shownSelection) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onSelect(null)
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [shownSelection, onSelect])

  // Read failures are announced once per outage; the panel keeps its shape.
  useReadFailureToast("nodes", nodes.isError, m.overview_nodes_failed())

  const stale = runtimes.isError && runtimes.dataUpdatedAt > 0 ? clockTime(runtimes.dataUpdatedAt) : null

  return (
    <main className="pointer-events-none fixed inset-0">
      <header className="absolute inset-x-0 top-0 flex items-center justify-between gap-3 p-4 md:p-5">
        <div className="enter">
          <NetworkChip
            name={network?.name ?? null}
            nodes={nodes.data ? scene.nodes.length : null}
            links={nodes.data ? linkSummary(observation, runtimes.data !== undefined) : null}
            stale={stale}
          />
        </div>
        <div className="enter glass pointer-events-auto flex h-11 items-center gap-0.5 rounded-full px-1.5" style={{ "--enter-delay": "80ms" }}>
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
          <NodeCard node={selected} nodes={byId} observation={observation} runtimes={runtimes.data ?? null} now={now} onSelect={onSelect} />
        </aside>
      ) : null}
    </main>
  )
}

function useReadFailureToast(id: string, failing: boolean, message: string) {
  const announced = useRef(false)
  useEffect(() => {
    if (failing && !announced.current) {
      announced.current = true
      toast.error(message, { id: `read-${id}` })
    }
    if (!failing) announced.current = false
  }, [failing, id, message])
}
