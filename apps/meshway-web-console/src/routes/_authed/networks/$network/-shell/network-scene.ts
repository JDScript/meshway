import { useQuery } from "@tanstack/react-query"
import { useMatch } from "@tanstack/react-router"
import { useEffect, useMemo } from "react"

import { nodesQuery } from "@/api/queries/nodes"
import { networkStateQuery } from "@/api/queries/state"
import { recentLinksOf, useHistory } from "@/api/queries/use-history"
import { EMPTY_SCENE } from "@/globe/scene"
import {
  linkKey,
  mapNodes,
  mergeRecentLinks,
  overviewScene,
  shortId,
} from "@/lib/network-map"
import { onGlobeSelect, setGlobe } from "@/state/globe"
import { useHistoryLayer } from "@/state/history-layer"

import { useNodeNavigation } from "../nodes/-navigation"

const HISTORY_FILTER = {
  mode: "recentLinks" as const,
  kinds: ["link_opened" as const, "link_closed" as const],
}

/**
 * What the globe shows of a network, on every one of its pages: the nodes
 * the controller located, the links their State reports, and the history
 * layer's links once a window is chosen. The overview reads the same to
 * state what the globe cannot show.
 */
export function useNetworkScene(networkId: string) {
  const state = useQuery(networkStateQuery(networkId))
  const inventory = useQuery(nodesQuery(networkId))
  const window = useHistoryLayer(networkId)
  const history = useHistory(networkId, window, HISTORY_FILTER)

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

  return {
    state,
    nodes,
    nameOf,
    scene,
    located,
    currentKeys,
    window,
    history,
    historyLinks,
  }
}

/**
 * Puts the network on the globe while its pages are open: its scene, the
 * node whose workspace is open in focus; a click on a node opens it.
 */
export function useGlobeNetwork(networkId: string) {
  const { scene } = useNetworkScene(networkId)
  const openNodeId =
    useMatch({
      from: "/_authed/networks/$network/nodes/$node",
      shouldThrow: false,
      select: (match) => match.params.node,
    }) ?? null

  useEffect(() => {
    setGlobe({ scene, selectedNodeId: openNodeId })
  }, [scene, openNodeId])
  useEffect(
    () => () => setGlobe({ scene: EMPTY_SCENE, selectedNodeId: null }),
    []
  )

  const { openNode } = useNodeNavigation(networkId)
  useEffect(
    () =>
      onGlobeSelect((nodeId) => {
        if (nodeId) openNode(nodeId)
      }),
    [openNode]
  )
}
