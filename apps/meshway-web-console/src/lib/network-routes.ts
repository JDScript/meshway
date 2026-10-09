import type {
  AggregatedPath,
  EvaluationOutcome,
  EvaluationResponse,
  TargetProblem,
} from "@meshway/control-client"

// Routes from one node to every other: the controller's plan for each
// destination (`evaluateConfiguration`, its main path first) and the paths
// the nodes report using now (`StateView.paths`). A plan is not a
// measurement, and current links are never stitched into a path.

export type RouteHop = {
  from: string
  to: string
  /** The `/topology/<i>` pointer that allows the edge; null under the default. */
  topology: string | null
}

export type NodeRoute =
  | { target: string; status: "pending" }
  | { target: string; status: "failed"; error: unknown }
  | {
      target: string
      status: "planned"
      /** The main planned path's hops, source first; empty without a path. */
      hops: RouteHop[]
      /** Access allows traffic from the source to the target. */
      authorized: boolean
      outcome: EvaluationOutcome
      problem: TargetProblem | null
    }

const pairKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)

export function plannedRoute(
  target: string,
  response: EvaluationResponse
): NodeRoute {
  const { evaluation } = response
  const path = evaluation.paths[0]
  const hops: RouteHop[] = []
  if (path)
    for (let at = 0; at + 1 < path.nodes.length; at++) {
      const from = path.nodes[at]
      const to = path.nodes[at + 1]
      if (from === undefined || to === undefined) continue
      const edge = path.edges.find(
        (item) => pairKey(item.a, item.b) === pairKey(from, to)
      )
      hops.push({ from, to, topology: edge?.topology ?? null })
    }
  return {
    target,
    status: "planned",
    hops,
    authorized: evaluation.access.allowed,
    outcome: evaluation.outcome,
    problem:
      evaluation.targets.find((item) => item.nodeId === target)?.problem ??
      null,
  }
}

/** The nodes between source and target. */
export const viaOf = (route: NodeRoute): string[] =>
  route.status === "planned" ? route.hops.slice(1).map((hop) => hop.from) : []

/** How many hops a planned path takes; null without one (pending, failed or no path). */
export const hopCount = (route: NodeRoute): number | null =>
  route.status === "planned" && route.hops.length > 0 ? route.hops.length : null

export type DirectedEdge = {
  key: string
  from: string
  to: string
  /** The undirected pair it runs along (`a|b`, sorted). */
  pair: string
  /** The destinations whose main path uses it. */
  targets: string[]
  /** Some destination using it is allowed by access. */
  authorized: boolean
}

/** Every hop of the planned main paths, once per direction. */
export function routeEdges(routes: readonly NodeRoute[]): DirectedEdge[] {
  const edges = new Map<string, DirectedEdge>()
  for (const route of routes) {
    if (route.status !== "planned") continue
    for (const hop of route.hops) {
      const key = `${hop.from}>${hop.to}`
      const edge = edges.get(key) ?? {
        key,
        from: hop.from,
        to: hop.to,
        pair: pairKey(hop.from, hop.to),
        targets: [],
        authorized: false,
      }
      edge.targets.push(route.target)
      edge.authorized ||= route.authorized
      edges.set(key, edge)
    }
  }
  return [...edges.values()]
}

/**
 * Where each node sits from the source: its hop count, and the node it is
 * reached through (its parent in the tree of main paths). Nodes without a
 * planned path are left out.
 */
export function routeTree(routes: readonly NodeRoute[]) {
  const depth = new Map<string, number>()
  const parent = new Map<string, string>()
  for (const route of routes) {
    const hops = hopCount(route)
    if (hops === null || route.status !== "planned") continue
    depth.set(route.target, hops)
    const last = route.hops.at(-1)
    if (last) parent.set(route.target, last.from)
  }
  return { depth, parent }
}

/**
 * The directions business paths reported now run along each pair: a hop
 * `from>to` per path step. Expired reports are left out; a relay seeing
 * ciphertext is still the path's step, nothing more.
 */
export function reportedFlows(
  paths: readonly AggregatedPath[],
  asOf: string
): Map<string, Set<string>> {
  const now = Date.parse(asOf)
  const flows = new Map<string, Set<string>>()
  for (const { state } of paths) {
    if (Date.parse(state.expiresAt) <= now) continue
    for (let at = 0; at + 1 < state.nodes.length; at++) {
      const from = state.nodes[at]
      const to = state.nodes[at + 1]
      if (from === undefined || to === undefined) continue
      const pair = pairKey(from, to)
      const directions = flows.get(pair) ?? new Set<string>()
      directions.add(`${from}>${to}`)
      flows.set(pair, directions)
    }
  }
  return flows
}
