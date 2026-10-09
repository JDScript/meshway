import type {
  ConfigurationView,
  NodeSettingsView,
  TopologyDefault,
  TransportProtocol,
} from "@meshway/control-client"

import { DIALER_PROTOCOLS } from "./dialer-model"

/** Bounded drawing window; a partial window is explicitly labelled in the view. */
export const TOPOLOGY_NODE_LIMIT = 80

export type DeclaredEdge = {
  key: string
  a: string
  b: string
  /** Null means the default declaration; otherwise the topology array index. */
  rule: number | null
  /** Null is unrestricted; [] explicitly allows no carrier. */
  transports: readonly TransportProtocol[] | null
}

export type DeclaredTopology = {
  defaultAction: TopologyDefault
  nodeIds: string[]
  totalNodes: number
  edges: DeclaredEdge[]
}

type Input = Pick<
  ConfigurationView,
  "nodes" | "groups" | "topology" | "defaults"
>

/** Read a selector's declared members from the non-secret configuration view.
 * This is a display projection, never an authorization or executable-path result.
 */
function declares(
  config: Input,
  selector: string,
  id: string,
  node: NodeSettingsView
): boolean {
  if (selector === "*") return true
  if (selector.startsWith("node:")) {
    const value = selector.slice(5).toLowerCase()
    return (
      value === id ||
      value === id.replaceAll("-", "") ||
      value === `{${id}}` ||
      value === `urn:uuid:${id}`
    )
  }
  if (node.identity.type === "tag") return node.identity.tags.includes(selector)
  if (selector === node.identity.user) return true
  const group = Object.hasOwn(config.groups, selector)
    ? config.groups[selector]
    : undefined
  return group?.includes(node.identity.user) ?? false
}

/** The adjacency declared by configuration, independent of State, access,
 * routing and transport readiness. The server remains the execution authority.
 */
export function configurationTopology(
  config: Input,
  selected?: string | null
): DeclaredTopology {
  const all = Object.entries(config.nodes).sort(([a], [b]) =>
    a.localeCompare(b)
  )
  const nodes = all.slice(0, TOPOLOGY_NODE_LIMIT)
  if (selected && !nodes.some(([id]) => id === selected)) {
    const focus = all.find(([id]) => id === selected)
    if (focus) nodes.splice(nodes.length - 1, 1, focus)
  }
  nodes.sort(([a], [b]) => a.localeCompare(b))
  const matches = new Map<string, Set<string>>()
  for (const rule of config.topology)
    for (const selector of [rule.a, rule.b]) {
      if (!matches.has(selector))
        matches.set(
          selector,
          new Set(
            nodes
              .filter(([id, node]) => declares(config, selector, id, node))
              .map(([id]) => id)
          )
        )
    }
  const edges: DeclaredEdge[] = []
  for (let i = 0; i < nodes.length; i++)
    for (let j = i + 1; j < nodes.length; j++) {
      const a = nodes[i]?.[0],
        b = nodes[j]?.[0]
      if (a === undefined || b === undefined) continue
      let blocked = false
      let allowing: number | null = null
      for (const [index, rule] of config.topology.entries()) {
        const left = matches.get(rule.a),
          right = matches.get(rule.b)
        if (!(
          (left?.has(a) && right?.has(b)) ||
          (left?.has(b) && right?.has(a))
        ))
          continue
        if (rule.action === "block") {
          blocked = true
          break
        }
        allowing ??= index
      }
      if (
        !blocked &&
        (allowing !== null || config.defaults.topology === "allow")
      )
        edges.push({
          key: `${a}|${b}`,
          a,
          b,
          rule: allowing,
          transports:
            allowing === null
              ? null
              : (config.topology[allowing]?.transports ?? null),
        })
    }
  return {
    defaultAction: config.defaults.topology,
    nodeIds: nodes.map(([id]) => id),
    totalNodes: all.length,
    edges,
  }
}

export type DialDirection = {
  from: string
  to: string
  /** What `from` may dial `to` with, in the edge's carrier order. */
  transports: TransportProtocol[]
  /** When none: the first of the edge, the dialer and the listener that leaves nothing. */
  missing: "edge" | "dialer" | "listener" | null
}

/**
 * Both dial directions of a declared edge as the configuration states them
 * (topology.md, carrier intersection): the edge's carriers, the dialer's
 * enabled dialers (or the defaults when it states none) and the listener's
 * enabled listeners. Stream, TLS, runtime capability and materials are the
 * server's to check; this never says a carrier works.
 */
export function edgeDirections(
  config: Pick<ConfigurationView, "nodes" | "defaults">,
  edge: Pick<DeclaredEdge, "a" | "b" | "transports">
): [DialDirection, DialDirection] {
  const order = edge.transports ?? DIALER_PROTOCOLS
  const dials = (id: string) => {
    const node = Object.hasOwn(config.nodes, id) ? config.nodes[id] : undefined
    const set = node?.dialers ?? config.defaults.dialers
    return new Set(
      DIALER_PROTOCOLS.filter((protocol) => {
        const entry = set[protocol]
        return entry != null && entry.enabled !== false
      })
    )
  }
  const listens = (id: string) => {
    const node = Object.hasOwn(config.nodes, id) ? config.nodes[id] : undefined
    return new Set(
      Object.values(node?.listeners ?? {})
        .filter((listener) => listener.enabled)
        .map((listener) => listener.protocol)
    )
  }
  const direction = (from: string, to: string): DialDirection => {
    const dialer = dials(from)
    const listener = listens(to)
    const transports = order.filter(
      (protocol) => dialer.has(protocol) && listener.has(protocol)
    )
    return {
      from,
      to,
      transports,
      missing:
        transports.length > 0
          ? null
          : order.length === 0
            ? "edge"
            : !order.some((protocol) => dialer.has(protocol))
              ? "dialer"
              : "listener",
    }
  }
  return [direction(edge.a, edge.b), direction(edge.b, edge.a)]
}
