import type { EvaluationResponse } from "@meshway/control-client"
import {
  useQueries,
  useQuery,
  type UseQueryResult,
} from "@tanstack/react-query"
import { useMatch, useNavigate } from "@tanstack/react-router"
import {
  createContext,
  use,
  useCallback,
  useEffect,
  useMemo,
  type ReactNode,
} from "react"

import { configurationQuery, isDenied } from "@/api/queries/configuration"
import { routeQuery } from "@/api/queries/evaluation"
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
  pairAdjacencies,
  reportedAdjacencies,
  type MapConnection,
} from "@/lib/network-map"
import {
  plannedRoute,
  reportedFlows,
  routeEdges,
  type NodeRoute,
} from "@/lib/network-routes"
import { stateOf } from "@/lib/nodes/state-model"
import { onGlobeSelect, setGlobe, useGlobeMarkersHidden } from "@/state/globe"
import { useHistoryLayer } from "@/state/history-layer"

import type { MapPage } from "../-map/selection"
import { useNodeNavigation } from "../nodes/-navigation"

const HISTORY_FILTER = {
  mode: "recentLinks" as const,
  kinds: ["link_opened" as const, "link_closed" as const],
}

/** The map page open, and what it has selected. */
function useMapRoute() {
  const overview = useMatch({
    from: "/_authed/networks/$network/overview",
    shouldThrow: false,
    select: (match) => match.search,
  })
  const topology = useMatch({
    from: "/_authed/networks/$network/topology",
    shouldThrow: false,
    select: (match) => match.search,
  })
  const workspace = useMatch({
    from: "/_authed/networks/$network/nodes/$node",
    shouldThrow: false,
    select: (match) => match.params.node,
  })
  const page: MapPage | null = topology
    ? "topology"
    : overview
      ? "overview"
      : null
  const search = topology ?? overview
  return {
    page,
    /** The node a map page has selected. */
    selected: search?.node ?? null,
    /** The node anything shows in focus: an open workspace's, else the map's. */
    focus: workspace ?? search?.node ?? null,
  }
}

/**
 * The controller's plan from `source` to every other drawn node, one
 * request per destination (kept per revision). Owners and admins only.
 */
function useRoutes(
  networkId: string,
  revision: number | undefined,
  source: string | null,
  targets: readonly string[]
): NodeRoute[] | null {
  const asked = revision !== undefined && source !== null
  const combine = useCallback(
    (results: UseQueryResult<EvaluationResponse>[]) =>
      targets.map((target, index): NodeRoute => {
        const result = results[index]
        if (result?.data) return plannedRoute(target, result.data)
        if (result?.isError)
          return { target, status: "failed", error: result.error }
        return { target, status: "pending" }
      }),
    [targets]
  )
  const routes = useQueries({
    queries: asked
      ? targets.map((target) => routeQuery(networkId, revision, source, target))
      : [],
    combine,
  })
  return asked ? routes : null
}

function useSceneModel(networkId: string) {
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
  const route = useMapRoute()
  const declared = useMemo(
    () =>
      configured
        ? configurationTopology(configured.configuration, route.focus)
        : undefined,
    [configured, route.focus]
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
  /** The topology's nodes: the configuration's drawing window when it is read. */
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
  const stateBehind =
    configured !== undefined &&
    runtimeState !== undefined &&
    runtimeState.configurationRevision !== configured.revision
  const links = stateBehind ? undefined : runtimeState?.links
  const waitingForConfiguration =
    canRead && !configured && !isDenied(configuration.error)
  /** The topology's edges: declared pairs with their runtime state. */
  const adjacencies = useMemo(
    () =>
      pairAdjacencies(
        reportedAdjacencies(stateBehind ? undefined : runtimeState)
      ),
    [stateBehind, runtimeState]
  )
  const edges = useMemo(
    () =>
      waitingForConfiguration
        ? []
        : mapConnections(links ?? [], historyLinks, declared, adjacencies),
    [waitingForConfiguration, links, historyLinks, declared, adjacencies]
  )
  /** What the nodes report now: connections, with history pairs apart. */
  const connections = useMemo(
    () => mapConnections(runtimeState?.links ?? [], historyLinks),
    [runtimeState, historyLinks]
  )
  const flows = useMemo(
    () =>
      runtimeState
        ? reportedFlows(runtimeState.paths, runtimeState.asOf)
        : new Map<string, Set<string>>(),
    [runtimeState]
  )

  const source =
    route.page !== null && route.selected !== null && declared
      ? route.selected
      : null
  const targets = useMemo(
    () =>
      source && declared ? declared.nodeIds.filter((id) => id !== source) : [],
    [source, declared]
  )
  const routes = useRoutes(networkId, configured?.revision, source, targets)

  // The globe draws current connections; with a node selected on the
  // overview, the hops of its planned paths too.
  const globeEdges = useMemo((): readonly MapConnection[] => {
    if (!routes || route.page !== "overview") return connections
    const byPair = new Map(connections.map((edge) => [edge.key, edge]))
    const pairs = new Set(routeEdges(routes).map((edge) => edge.pair))
    return [
      ...connections.filter((edge) => !pairs.has(edge.key)),
      ...[...pairs].map(
        (pair): MapConnection =>
          byPair.get(pair) ?? {
            key: pair,
            a: pair.split("|")[0] ?? "",
            b: pair.split("|")[1] ?? "",
            look: "allowed",
            status: "allowed",
            transports: [],
            inUse: false,
            channels: [],
            adjacency: null,
          }
      ),
    ]
  }, [route.page, routes, connections])
  const scene = useMemo(
    () => overviewScene(nodes, waitingForConfiguration ? [] : globeEdges),
    [nodes, waitingForConfiguration, globeEdges]
  )
  const located = useMemo(
    () =>
      new Set(
        scene.nodes.filter((node) => node.placement).map((node) => node.id)
      ),
    [scene]
  )
  /** No State yet: runtime facts (status, connections) are not known, not absent. */
  const stateLoading = state.data === undefined && !state.isError
  return {
    state,
    stateLoading,
    /** The node list is not read yet: the configuration's, else the inventory (with State's peers). */
    nodesLoading: canRead
      ? waitingForConfiguration
      : (inventory.data === undefined && !inventory.isError) || stateLoading,
    /** Some planned path from the selected node is still being asked. */
    routesPending: routes?.some((route) => route.status === "pending") ?? false,
    stateBehind,
    inventory: inventory.data?.items,
    canRead,
    configuration: configured,
    nodes,
    drawnNodes,
    edges,
    connections,
    flows,
    routes,
    declared,
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

export type NetworkScene = ReturnType<typeof useSceneModel>

const SceneContext = createContext<NetworkScene | null>(null)

/**
 * What a network's pages show of it, read once for all of them: the nodes
 * where geojs places them, the declared topology with what the nodes report,
 * the history layer once a window is chosen, and with a node selected on a
 * map page, the planned paths from it.
 */
export function NetworkSceneProvider({
  networkId,
  children,
}: {
  networkId: string
  children: ReactNode
}) {
  const scene = useSceneModel(networkId)
  return <SceneContext value={scene}>{children}</SceneContext>
}

export function useNetworkScene(): NetworkScene {
  const scene = use(SceneContext)
  if (!scene) throw new Error("useNetworkScene outside NetworkSceneProvider")
  return scene
}

/**
 * Puts the network on the globe while its pages are open: its scene, the
 * node in focus; on the topology the globe stays behind the diagram. A
 * click on a node selects it on the overview and opens it elsewhere.
 */
export function useGlobeNetwork(networkId: string) {
  const { scene } = useNetworkScene()
  const navigate = useNavigate()
  const route = useMapRoute()
  const handedOver = useGlobeMarkersHidden()

  useEffect(() => {
    const topology = route.page === "topology"
    setGlobe({
      scene: handedOver ? { ...scene, links: [] } : scene,
      // Behind the topology the globe holds still; the diagram has the focus.
      selectedNodeId: topology ? null : route.focus,
    })
  }, [scene, handedOver, route.focus, route.page])
  useEffect(
    () => () =>
      setGlobe({
        scene: EMPTY_SCENE,
        selectedNodeId: null,
        backdrop: null,
        disc: null,
      }),
    []
  )

  const { openNode } = useNodeNavigation(networkId)
  const overview = route.page === "overview"
  useEffect(
    () =>
      onGlobeSelect((nodeId, group) => {
        if (overview) {
          const many = group && group.length > 1
          void navigate({
            to: "/networks/$network/overview",
            params: { network: networkId },
            search: many
              ? { group: [...group] }
              : { node: nodeId ?? undefined },
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
