import type { LinkLook } from "@/globe/scene"
import type { MapConnection, MapNode } from "@/lib/network-map"
import { routeEdges, routeTree, type NodeRoute } from "@/lib/network-routes"
import type { NodeStatus } from "@/lib/nodes/status"
import {
  focusLayout,
  overviewLayout,
  type Box,
  type Point,
  type RouteTree,
} from "@/lib/topology-layout"

// What the topology draws: every node at its place, and either the
// declared edges with what the nodes report (nothing selected), or the
// planned paths from the selected node, each hop pointing the way traffic
// goes. A selected link is drawn in either view.

export type DiagramNode = {
  id: string
  name: string
  status: NodeStatus
  point: Point
  selected: boolean
  /** Off the paths of the selected node (or not reached by any). */
  dim: boolean
}

export type DiagramEdge = {
  key: string
  a: string
  b: string
  /** Set on a planned path's hop: the way it points. */
  direction: { from: string; to: string } | null
  connection: MapConnection | null
  look: LinkLook
  /** Directions business paths reported now take along it (`from>to`); empty unless it is in use. */
  flows: { from: string; to: string }[]
  /** The selected link. */
  selected: boolean
  /** Planned only for destinations access refuses. */
  unauthorized: boolean
}

export type Diagram = {
  nodes: DiagramNode[]
  edges: DiagramEdge[]
  focused: boolean
}

/** Nodes in the order of their longitude (so the ring reads like the globe), unplaced ones after by name. */
export function placeOrder(nodes: readonly MapNode[]): MapNode[] {
  const longitude = (node: MapNode) =>
    node.geo?.kind === "found" ? (node.geo.geo.position?.lng ?? null) : null
  return [...nodes].sort((x, y) => {
    const a = longitude(x)
    const b = longitude(y)
    if (a !== null && b !== null && a !== b) return a - b
    if (a !== null && b === null) return -1
    if (a === null && b !== null) return 1
    return x.name.localeCompare(y.name) || x.id.localeCompare(y.id)
  })
}

const pairKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)

function flowsOf(
  connection: MapConnection | null,
  flows: ReadonlyMap<string, ReadonlySet<string>>
) {
  if (!connection?.inUse) return []
  return [...(flows.get(pairKey(connection.a, connection.b)) ?? [])].flatMap(
    (step) => {
      const [from, to] = step.split(">")
      return from !== undefined && to !== undefined ? [{ from, to }] : []
    }
  )
}

/** Without planned paths (a member): the selected node's current neighbours, one hop out. */
function neighbourTree(
  source: string,
  connections: readonly MapConnection[]
): RouteTree {
  const depth = new Map<string, number>()
  const parent = new Map<string, string>()
  for (const edge of connections) {
    if (edge.status === "history") continue
    const peer = edge.a === source ? edge.b : edge.b === source ? edge.a : null
    if (peer === null) continue
    depth.set(peer, 1)
    parent.set(peer, source)
  }
  return { depth, parent }
}

export function topologyDiagram({
  nodes,
  edges,
  routes,
  flows,
  selected,
  link,
  box,
}: {
  nodes: readonly MapNode[]
  /** Declared edges with their runtime state; without a declaration, what the nodes report. */
  edges: readonly MapConnection[]
  /** Planned paths from `selected`; null where they are not read (a member). */
  routes: readonly NodeRoute[] | null
  flows: ReadonlyMap<string, ReadonlySet<string>>
  selected: string | null
  link: readonly [string, string] | null
  box: Box
}): Diagram {
  const ordered = placeOrder(nodes)
  const ids = ordered.map((node) => node.id)
  const source = selected !== null && ids.includes(selected) ? selected : null
  const byPair = new Map(edges.map((edge) => [edge.key, edge]))
  const selectedPair = link ? pairKey(link[0], link[1]) : null

  const base = (edge: MapConnection): DiagramEdge => ({
    key: edge.key,
    a: edge.a,
    b: edge.b,
    direction: null,
    connection: edge,
    look: edge.look,
    flows: flowsOf(edge, flows),
    selected: edge.key === selectedPair,
    unauthorized: false,
  })

  let tree: RouteTree | null = null
  let drawn: DiagramEdge[]
  if (source === null) drawn = edges.map(base)
  else if (routes) {
    tree = routeTree(routes)
    drawn = routeEdges(routes).map((hop) => {
      const connection = byPair.get(hop.pair) ?? null
      const [a, b] = hop.pair.split("|")
      return {
        key: hop.key,
        a: a ?? hop.from,
        b: b ?? hop.to,
        direction: { from: hop.from, to: hop.to },
        connection,
        look: connection?.look ?? "allowed",
        flows: flowsOf(connection, flows),
        selected: hop.pair === selectedPair,
        unauthorized: !hop.authorized,
      }
    })
  } else {
    tree = neighbourTree(source, edges)
    drawn = edges
      .filter(
        (edge) =>
          edge.status !== "history" && (edge.a === source || edge.b === source)
      )
      .map(base)
  }
  if (
    link &&
    selectedPair !== null &&
    !drawn.some((edge) => pairKey(edge.a, edge.b) === selectedPair)
  ) {
    const edge = byPair.get(selectedPair)
    if (edge) drawn.push(base(edge))
  }

  const points =
    source !== null && tree
      ? focusLayout(source, ids, tree, box)
      : overviewLayout(ids, box)
  const onPaths = new Set(drawn.flatMap((edge) => [edge.a, edge.b]))
  return {
    focused: source !== null,
    nodes: ordered.flatMap((node) => {
      const point = points.get(node.id)
      return point
        ? [
            {
              id: node.id,
              name: node.name,
              status: node.status,
              point,
              selected: node.id === source,
              dim:
                source !== null && node.id !== source && !onPaths.has(node.id),
            },
          ]
        : []
    }),
    edges: drawn,
  }
}
