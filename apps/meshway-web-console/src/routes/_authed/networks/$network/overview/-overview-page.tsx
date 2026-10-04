import type { CSSProperties } from "react"

import {
  EDGE,
  MOBILE_EDGE,
  MOBILE_SHEET_BOTTOM,
  PANEL_TOP,
} from "@/components/surface/layout"
import { Surface } from "@/components/surface/surface"
import { useIsMobile } from "@/hooks/use-mobile"
import { useBooted } from "@/state/boot"
import { usePointedNode } from "@/state/globe"
import { chooseHistoryLayer } from "@/state/history-layer"
import { useLive } from "@/state/live"

import { useNetworkScene } from "../-shell/network-scene"
import { useNodeNavigation } from "../nodes/-navigation"
import { NodeCard } from "./-node-card"
import { StatusCard } from "./-status-card"

/**
 * The overview: in a corner over the globe, what the globe cannot show
 * (counts, unlocated nodes, links with an unlocated end, the optional
 * history layer), and the node the pointer is on. The globe itself shows
 * the network on every page (`useGlobeNetwork`).
 */
export function OverviewPage({ networkId }: { networkId: string }) {
  const mobile = useIsMobile()
  const shown = useBooted()
  const live = useLive(networkId)
  const scene = useNetworkScene(networkId)
  const { state, nodes, located, nameOf } = scene
  const navigation = useNodeNavigation(networkId)

  const pointed = usePointedNode()
  const pointedNode =
    pointed !== null && located.has(pointed)
      ? (nodes.find((node) => node.id === pointed) ?? null)
      : null

  const place: CSSProperties = mobile
    ? {
        left: MOBILE_EDGE,
        right: MOBILE_EDGE,
        bottom: MOBILE_SHEET_BOTTOM,
        maxHeight: "55dvh",
      }
    : {
        right: EDGE,
        bottom: EDGE,
        width: 340,
        maxHeight: `calc(100dvh - ${PANEL_TOP + EDGE}px)`,
      }

  return (
    <div
      className="pointer-events-none absolute flex flex-col justify-end gap-3"
      style={place}
    >
      {pointedNode && shown && !mobile ? (
        <div className="glass pointer-events-auto shrink-0 rounded-surface">
          <NodeCard node={pointedNode} />
        </div>
      ) : null}
      <Surface
        shown={shown}
        from={mobile ? "bottom" : "right"}
        className="glass pointer-events-auto flex min-h-0 flex-col overflow-hidden rounded-surface"
      >
        <StatusCard
          state={state.data}
          stateError={state.isError ? state.error : null}
          nodes={nodes}
          located={located}
          live={live}
          nameOf={nameOf}
          onOpenNode={(nodeId) => navigation.openNode(nodeId)}
          mobile={mobile}
          history={{
            window: scene.window,
            onChoose: (choice) => chooseHistoryLayer(networkId, choice),
            history: scene.history,
            links: scene.historyLinks,
            located,
            currentKeys: scene.currentKeys,
            nameOf,
          }}
        />
      </Surface>
    </div>
  )
}
