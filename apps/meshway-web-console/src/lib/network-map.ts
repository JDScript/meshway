import type {
  AggregatedLink,
  LocationAbsentReason,
  NodeInventoryView,
  RecentLink,
  StateView,
} from "@meshway/control-client"

import type { LinkLook, Scene, SceneLink, SceneNode } from "@/globe/scene"
import {
  nodeAddresses,
  type AddressGeo,
  type AddressScope,
  type GeoOf,
  type LocationAddress,
} from "@/lib/geo"
import { nodeStatus, type NodeStatus } from "@/lib/nodes/status"

// The overview's reading of the network's State: which nodes are located,
// which are not and why, and which links can be drawn between located ends.
// A node is located where geojs places the address the server rests its
// location on. Nothing here is estimated: a node without that position is
// unlocated, a link with an unlocated end is counted, never drawn at an
// invented place.

/**
 * Why a node is not on the globe: the server's `locationAbsent`; that this
 * browser does not look up locations, is looking it up or could not; or
 * `noPosition` when geojs names a place but no coordinates.
 */
export type Unlocated =
  | LocationAbsentReason
  | "lookupOff"
  | "lookupPending"
  | "lookupFailed"
  | "noPosition"
  | "unknown"

export type MapNode = {
  id: string
  name: string
  status: NodeStatus
  /** The address the server rests its location on. */
  location: LocationAddress | null
  /** Where geojs places that address, or why that is not known. */
  geo: AddressGeo | null
  /** Set exactly when the node is not on the globe. */
  unlocated: Unlocated | null
  receivedAt: string | null
  lastAuthenticatedAt: string | null
}

/** The addresses the nodes' locations rest on (all public): what the globe looks up. */
export function locationAddresses(state: StateView): Map<string, AddressScope> {
  return new Map(
    Object.values(state.nodes).flatMap((view) => {
      const { location } = nodeAddresses(view.derived)
      return location ? [[location.address, "public" as const]] : []
    })
  )
}

function unlocatedOf(
  location: LocationAddress | null,
  locationAbsent: LocationAbsentReason | null,
  geo: AddressGeo | null
): Unlocated | null {
  if (!location) return locationAbsent ?? "unknown"
  switch (geo?.kind) {
    case "found":
      return geo.geo.position ? null : "noPosition"
    case "pending":
      return "lookupPending"
    case "failed":
      return "lookupFailed"
    case "off":
      return "lookupOff"
    case "notPublic":
    case undefined:
      return "unknown"
  }
}

/**
 * Every node in the State, named by its configured hostname; a node the
 * account cannot list (a peer of its own device) takes its status from its
 * report alone. `geoOf` says where the addresses of `locationAddresses` are.
 */
export function mapNodes(
  state: StateView,
  inventory: ReadonlyMap<string, NodeInventoryView>,
  geoOf: GeoOf
): MapNode[] {
  return [...new Set([...Object.keys(state.nodes), ...inventory.keys()])]
    .map((id): MapNode => {
      const view = Object.hasOwn(state.nodes, id) ? state.nodes[id] : undefined
      const { location, locationAbsent } = view
        ? nodeAddresses(view.derived)
        : { location: null, locationAbsent: null }
      const geo = location ? geoOf(location.address) : null
      const listed = inventory.get(id) ?? null
      return {
        id,
        name: listed?.hostname ?? view?.hostname ?? id.slice(0, 8),
        status: nodeStatus(listed, view),
        location,
        geo,
        unlocated: unlocatedOf(location, locationAbsent, geo),
        receivedAt: view?.receivedAt ?? null,
        lastAuthenticatedAt: view?.lastAuthenticatedAt ?? null,
      }
    })
    .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
}

function sceneNode(node: MapNode): SceneNode {
  const position = node.geo?.kind === "found" ? node.geo.geo.position : null
  return {
    id: node.id,
    name: node.name,
    placement: position
      ? {
          lat: position.lat,
          lng: position.lng,
          accuracyKm: position.accuracyKm,
        }
      : null,
    status: node.status,
  }
}

/** How a current link looks: evidence problems first, then the reported phase. */
export function linkLook(link: AggregatedLink): LinkLook {
  if (link.evidence === "expired") return "expired"
  if (link.evidence === "conflicting") return "conflicting"
  switch (link.phase) {
    case "authenticated":
      return link.evidence === "matched" ? "matched" : "one_sided"
    case "preparing":
      return "preparing"
    case "failed":
      return "failed"
    case "closed":
      return "closed"
  }
}

export const linkKey = (link: { a: string; b: string; linkId: string }) =>
  `${link.a}|${link.b}|${link.linkId}`

/** Links with both ends located are drawn; the others are only counted and listed. */
const drawnBetween = <T extends { a: string; b: string }>(
  links: readonly T[],
  located: ReadonlySet<string>
): T[] => links.filter((link) => located.has(link.a) && located.has(link.b))

/** How many of each value, in `order`, leaving out those with none. */
export function countBy<T extends string>(
  values: readonly T[],
  order: readonly T[]
): { value: T; count: number }[] {
  const counts = new Map<T, number>()
  for (const value of values) counts.set(value, (counts.get(value) ?? 0) + 1)
  return order.flatMap((value) => {
    const count = counts.get(value) ?? 0
    return count > 0 ? [{ value, count }] : []
  })
}

/**
 * One channel seen in the history window. Both ends may record events for
 * the same link; they are one entry here, its latest event deciding whether
 * it was last seen open or closed.
 */
export type HistoryLink = {
  a: string
  b: string
  linkId: string
  transport: string
  lastEventAt: string
  closed: boolean
  /** The nodes whose recorded events this rests on. */
  sources: string[]
}

export function mergeRecentLinks(items: readonly RecentLink[]): HistoryLink[] {
  const merged = new Map<string, HistoryLink>()
  for (const item of items) {
    const [a, b] =
      item.sourceNodeId < item.peerNodeId
        ? [item.sourceNodeId, item.peerNodeId]
        : [item.peerNodeId, item.sourceNodeId]
    const key = linkKey({ a, b, linkId: item.linkId })
    const existing = merged.get(key)
    if (!existing) {
      merged.set(key, {
        a,
        b,
        linkId: item.linkId,
        transport: item.transport,
        lastEventAt: item.lastEventAt,
        closed: item.closed,
        sources: [item.sourceNodeId],
      })
      continue
    }
    if (!existing.sources.includes(item.sourceNodeId))
      existing.sources.push(item.sourceNodeId)
    if (Date.parse(item.lastEventAt) > Date.parse(existing.lastEventAt)) {
      existing.lastEventAt = item.lastEventAt
      existing.closed = item.closed
      existing.transport = item.transport
    }
  }
  return [...merged.values()].sort(
    (x, y) => Date.parse(y.lastEventAt) - Date.parse(x.lastEventAt)
  )
}

/**
 * The globe's scene: located nodes, current links between them, and (when
 * the history layer is on) history links between them that are not current.
 * Links between the same two nodes take successive lanes.
 */
export function overviewScene(
  nodes: readonly MapNode[],
  links: readonly AggregatedLink[],
  history: readonly HistoryLink[] | null
): Scene {
  const scene = nodes.map(sceneNode)
  const located = new Set(
    scene.filter((node) => node.placement).map((node) => node.id)
  )
  const current = drawnBetween(links, located)
  const currentKeys = new Set(links.map(linkKey))
  const past = history
    ? drawnBetween(
        history.filter((link) => !currentKeys.has(linkKey(link))),
        located
      )
    : []
  const lanes = new Map<string, number>()
  const lane = (a: string, b: string) => {
    const pair = `${a}|${b}`
    const next = lanes.get(pair) ?? 0
    lanes.set(pair, next + 1)
    return next
  }
  const sceneLinks: SceneLink[] = [
    ...[...current]
      .sort((x, y) => x.linkId.localeCompare(y.linkId))
      .map((link) => ({
        key: linkKey(link),
        nodeIds: [link.a, link.b] satisfies [string, string],
        look: linkLook(link),
        lane: lane(link.a, link.b),
      })),
    ...past.map((link) => ({
      key: `history|${linkKey(link)}`,
      nodeIds: [link.a, link.b] satisfies [string, string],
      look: "history" as const,
      lane: lane(link.a, link.b),
    })),
  ]
  return { nodes: scene, links: sceneLinks }
}
