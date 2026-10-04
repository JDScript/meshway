import { useQuery } from "@tanstack/react-query"
import { useMatch } from "@tanstack/react-router"
import { useEffect, useMemo, useState, type CSSProperties } from "react"

import { EMPTY_SCENE } from "@/features/globe/scene"
import { recentLinksOf, useHistory } from "@/features/history/use-history"
import {
  historyWindow,
  type HistoryWindow,
  type WindowChoice,
} from "@/features/history/window"
import { useNodeNavigation, useNodesRoute } from "@/features/nodes/navigation"
import { nodesQuery } from "@/features/nodes/queries"
import {
  EDGE,
  MOBILE_EDGE,
  MOBILE_SHEET_BOTTOM,
  PANEL_TOP,
} from "@/features/shell/layout"
import { Surface } from "@/features/shell/surface"
import { networkStateQuery } from "@/features/state/queries"
import { useIsMobile } from "@/hooks/use-mobile"
import { useBooted } from "@/state/boot"
import {
  highlightOnGlobe,
  onGlobeHover,
  onGlobeSelect,
  setGlobe,
} from "@/state/globe"
import { useLive } from "@/state/live"

import {
  linkKey,
  mapNodes,
  mergeRecentLinks,
  overviewScene,
  shortId,
} from "./model"
import { NodeCard } from "./node-card"
import { StatusCard } from "./status-card"

const OVERVIEW = "/_authed/networks/$network/overview" as const

const HISTORY_FILTER = {
  mode: "recentLinks" as const,
  kinds: ["link_opened" as const, "link_closed" as const],
}

/**
 * The overview: the globe with the nodes the controller located and the
 * links their State reports, and in a corner what the globe cannot show
 * (counts, unlocated nodes, links with an unlocated end, the optional
 * history layer). It writes the network's scene to the globe on every page
 * of the network; its own surfaces show on the overview only.
 */
export function OverviewPage({ networkId }: { networkId: string }) {
  const mobile = useIsMobile()
  const ready = useBooted()
  const current =
    useMatch({ from: OVERVIEW, shouldThrow: false, select: () => true }) ??
    false

  const state = useQuery(networkStateQuery(networkId))
  const inventory = useQuery(nodesQuery(networkId))
  const live = useLive(networkId)

  const names = useMemo(
    () =>
      new Map(
        (inventory.data?.items ?? []).map((node) => [node.id, node.hostname])
      ),
    [inventory.data]
  )
  const nameOf = useMemo(
    () => (id: string) => names.get(id) ?? shortId(id),
    [names]
  )
  const nodes = useMemo(
    () => (state.data ? mapNodes(state.data, names) : []),
    [state.data, names]
  )

  // Off by default; a chosen window belongs to the network it was chosen on.
  const [chosen, setChosen] = useState<{
    networkId: string
    window: HistoryWindow
  } | null>(null)
  const window = chosen?.networkId === networkId ? chosen.window : null
  const choose = (choice: WindowChoice | null) =>
    setChosen(
      choice ? { networkId, window: historyWindow(choice, Date.now()) } : null
    )
  const history = useHistory(networkId, window, HISTORY_FILTER)
  const historyLinks = useMemo(
    () => (window ? mergeRecentLinks(recentLinksOf(history.pages)) : null),
    [window, history.pages]
  )

  const links = state.data?.links
  const scene = useMemo(
    () => overviewScene(nodes, links ?? [], historyLinks),
    [nodes, links, historyLinks]
  )
  const located = useMemo(
    () =>
      new Set(
        scene.nodes.filter((node) => node.placement).map((node) => node.id)
      ),
    [scene]
  )
  const currentKeys = useMemo(
    () => new Set((links ?? []).map(linkKey)),
    [links]
  )

  // The globe focuses the node whose workspace is open.
  const { node: openNodeId } = useNodesRoute()
  useEffect(() => {
    setGlobe({ scene, selectedNodeId: openNodeId })
  }, [scene, openNodeId])
  useEffect(
    () => () => setGlobe({ scene: EMPTY_SCENE, selectedNodeId: null }),
    []
  )

  const navigation = useNodeNavigation(networkId)
  useEffect(
    () =>
      onGlobeSelect((nodeId) => {
        if (nodeId) navigation.openNode(nodeId)
      }),
    [navigation]
  )
  const [hovered, setHovered] = useState<string | null>(null)
  useEffect(() => onGlobeHover(setHovered), [])
  useEffect(() => {
    highlightOnGlobe(hovered)
  }, [hovered])
  const hoveredNode =
    hovered !== null && located.has(hovered)
      ? (nodes.find((node) => node.id === hovered) ?? null)
      : null

  const shown = ready && current
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
      {hoveredNode && shown && !mobile ? (
        <div className="glass pointer-events-auto shrink-0 rounded-surface">
          <NodeCard node={hoveredNode} />
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
            window,
            onChoose: choose,
            history,
            links: historyLinks,
            located,
            currentKeys,
            nameOf,
          }}
        />
      </Surface>
    </div>
  )
}
