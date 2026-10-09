import { useQuery } from "@tanstack/react-query"
import { useMatch, useNavigate } from "@tanstack/react-router"
import { useEffect, useMemo } from "react"

import { configurationQuery, isDenied } from "@/api/queries/configuration"
import { useAddressGeo } from "@/api/queries/ip-geo"
import { networkQuery } from "@/api/queries/network"
import { nodesQuery } from "@/api/queries/nodes"
import { networkStateQuery } from "@/api/queries/state"
import { recentLinksOf, useHistory } from "@/api/queries/use-history"
import { EMPTY_SCENE } from "@/globe/scene"
import { configurationTopology } from "@/lib/configuration/topology-view"
import { shortId } from "@/lib/format"
import {
  locationAddresses,
  mapConnections,
  mapNodes,
  mergeRecentLinks,
  overviewScene,
} from "@/lib/network-map"
import { stateOf } from "@/lib/nodes/state-model"
import { onGlobeSelect, setGlobe } from "@/state/globe"
import { useHistoryLayer } from "@/state/history-layer"
import { useOverviewView } from "@/state/overview-view"

import { useNodeNavigation } from "../nodes/-navigation"

const HISTORY_FILTER = {
  mode: "recentLinks" as const,
  kinds: ["link_opened" as const, "link_closed" as const],
}

/**
 * What the globe shows of a network, on every one of its pages: the nodes
 * where geojs places them, the links their State reports, and the history
 * layer's links once a window is chosen. The overview reads the same to
 * state what the globe cannot show.
 */
export function useNetworkScene(networkId: string) {
  const state = useQuery(networkStateQuery(networkId))
  const inventory = useQuery(nodesQuery(networkId))
  const network = useQuery(networkQuery(networkId))
  const canRead = network.data?.network.actions.readConfiguration === true
  const configuration = useQuery({
    ...configurationQuery(networkId),
    enabled: canRead,
  })
  const configured =
    canRead && !isDenied(configuration.error) ? configuration.data : undefined
  const overviewNode = useMatch({
    from: "/_authed/networks/$network/overview",
    shouldThrow: false,
    select: (match) => match.search.node,
  })
  const workspaceNode = useMatch({
    from: "/_authed/networks/$network/nodes/$node",
    shouldThrow: false,
    select: (match) => match.params.node,
  })
  const focus = workspaceNode ?? overviewNode
  const declared = useMemo(
    () =>
      configured
        ? configurationTopology(configured.configuration, focus)
        : undefined,
    [configured, focus]
  )
  const runtimeState = state.isError ? undefined : state.data
  const window = useHistoryLayer(networkId)
  const history = useHistory(networkId, window, HISTORY_FILTER)

  const listed = useMemo(
    () => new Map((inventory.data?.items ?? []).map((node) => [node.id, node])),
    [inventory.data]
  )
  const nameOf = useMemo(
    () => (id: string) =>
      configured?.configuration.nodes[id]?.hostname ??
      listed.get(id)?.hostname ??
      stateOf(state.data, id)?.hostname ??
      shortId(id),
    [listed, state.data, configured]
  )
  const addresses = useMemo(
    () => (state.data ? locationAddresses(state.data) : new Map()),
    [state.data]
  )
  const geoOf = useAddressGeo(addresses)
  const nodes = useMemo(
    () =>
      mapNodes(runtimeState, listed, geoOf, configured?.configuration.nodes),
    [runtimeState, listed, geoOf, configured]
  )
  const drawnNodes = useMemo(
    () =>
      declared
        ? nodes.filter((node) => declared.nodeIds.includes(node.id))
        : nodes,
    [declared, nodes]
  )
  const historyLinks = useMemo(
    () => (window ? mergeRecentLinks(recentLinksOf(history.pages)) : null),
    [window, history.pages]
  )
  // A different configuration revision cannot annotate the saved declaration
  // as current. Configuration edges remain visible while State catches up.
  const links =
    configured && runtimeState?.configurationRevision !== configured.revision
      ? undefined
      : runtimeState?.links
  const waitingForConfiguration =
    canRead && !configured && !isDenied(configuration.error)
  const edges = useMemo(
    () =>
      waitingForConfiguration
        ? []
        : mapConnections(links ?? [], historyLinks, declared),
    [waitingForConfiguration, links, historyLinks, declared]
  )
  const scene = useMemo(
    () =>
      overviewScene(
        drawnNodes,
        waitingForConfiguration ? [] : (links ?? []),
        waitingForConfiguration ? null : historyLinks,
        declared
      ),
    [drawnNodes, waitingForConfiguration, links, historyLinks, declared]
  )
  const located = useMemo(
    () =>
      new Set(
        scene.nodes.filter((node) => node.placement).map((node) => node.id)
      ),
    [scene]
  )
  return {
    state,
    inventory: inventory.data?.items,
    nodes,
    drawnNodes,
    edges,
    declared,
    configurationRevision: configured?.revision,
    waitingForConfiguration,
    configurationError:
      canRead && configuration.isError ? configuration.error : null,
    nameOf,
    scene,
    located,
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
  const rememberedView = useOverviewView()
  const navigate = useNavigate()
  const overview = useMatch({
    from: "/_authed/networks/$network/overview",
    shouldThrow: false,
    select: (match) => ({ node: match.search.node, view: match.search.view }),
  })
  const openNodeId =
    useMatch({
      from: "/_authed/networks/$network/nodes/$node",
      shouldThrow: false,
      select: (match) => match.params.node,
    }) ?? null

  useEffect(() => {
    setGlobe({
      scene,
      selectedNodeId: openNodeId ?? overview?.node ?? null,
      hidden:
        overview !== undefined &&
        (overview.view ?? rememberedView) === "topology",
    })
  }, [scene, openNodeId, overview, rememberedView])
  useEffect(
    () => () =>
      setGlobe({ scene: EMPTY_SCENE, selectedNodeId: null, hidden: false }),
    []
  )

  const { openNode } = useNodeNavigation(networkId)
  useEffect(
    () =>
      onGlobeSelect((nodeId, group) => {
        if (overview !== undefined) {
          void navigate({
            to: "/networks/$network/overview",
            params: { network: networkId },
            search: {
              view: overview.view,
              node:
                group && group.length > 1 ? undefined : (nodeId ?? undefined),
              group: group && group.length > 1 ? [...group] : undefined,
            },
            replace: true,
          })
          return true
        }
        if (group && group.length > 1) return false
        if (nodeId) openNode(nodeId)
        return true
      }),
    [openNode, overview, navigate, networkId]
  )
}
