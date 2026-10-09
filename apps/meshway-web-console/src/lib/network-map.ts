import type {
  AggregatedLink,
  LocationAbsentReason,
  NodeInventoryView,
  NodeSettingsView,
  RecentLink,
  StateView,
} from "@meshway/control-client"

import type { LinkLook, Scene, SceneLink, SceneNode } from "@/globe/scene"
import type {
  DeclaredEdge,
  DeclaredTopology,
} from "@/lib/configuration/topology-view"
import {
  nodeAddresses,
  type AddressGeo,
  type AddressScope,
  type GeoOf,
  type LocationAddress,
} from "@/lib/geo"
import {
  groupNodePairs,
  neighborConnections,
  type ConnectionStatus,
  type NeighborConnection,
} from "@/lib/nodes/connections"
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
 * Configuration nodes when readable, otherwise the visible inventory/State,
 * named by the configuration's hostname; a node the
 * account cannot list (a peer of its own device) takes its status from its
 * report alone. `geoOf` says where the addresses of `locationAddresses` are.
 */
export function mapNodes(
  state: StateView | undefined,
  inventory: ReadonlyMap<string, NodeInventoryView>,
  geoOf: GeoOf,
  configured?: Readonly<Record<string, NodeSettingsView>>
): MapNode[] {
  const ids = configured
    ? Object.keys(configured)
    : [...new Set([...Object.keys(state?.nodes ?? {}), ...inventory.keys()])]
  return ids
    .map((id): MapNode => {
      const view =
        state && Object.hasOwn(state.nodes, id) ? state.nodes[id] : undefined
      const { location, locationAbsent } = view
        ? nodeAddresses(view.derived)
        : { location: null, locationAbsent: null }
      const geo = location ? geoOf(location.address) : null
      const listed = inventory.get(id) ?? null
      return {
        id,
        name:
          configured?.[id]?.hostname ??
          listed?.hostname ??
          view?.hostname ??
          id.slice(0, 8),
        status:
          configured?.[id]?.enabled === false
            ? "disabled"
            : nodeStatus(listed, view),
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

/** Product adjacency styling; report provenance stays in channel details. */
export function connectionLook(connection: NeighborConnection): LinkLook {
  switch (connection.status) {
    case "connected":
      return "matched"
    case "connecting":
      return "preparing"
    case "failed":
      return "failed"
    case "unknown":
      return "expired"
  }
}

export const linkKey = (link: { a: string; b: string; linkId: string }) =>
  `${link.a}|${link.b}|${link.linkId}`

/** Edges with both ends located can also be drawn on the globe. */
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
 * Each adjacent pair has one edge, using the same model as the topology/list.
 */
export function overviewScene(
  nodes: readonly MapNode[],
  links: readonly AggregatedLink[],
  history: readonly HistoryLink[] | null,
  declared?: DeclaredTopology
): Scene {
  const scene = nodes.map(sceneNode)
  const located = new Set(
    scene.filter((node) => node.placement).map((node) => node.id)
  )
  const sceneLinks: SceneLink[] = drawnBetween(
    mapConnections(links, history, declared),
    located
  ).map((link) => ({
    key: link.key,
    nodeIds: [link.a, link.b] satisfies [string, string],
    look: link.look,
    lane: 0,
  }))
  return { nodes: scene, links: sceneLinks }
}

export type MapConnection = {
  key: string
  a: string
  b: string
  look: LinkLook
  status: ConnectionStatus | "history" | "allowed" | "unavailable"
  transports: string[]
  declaration?: DeclaredEdge
}

/** Shared graph projection: one current edge per adjacent pair, optional historical pairs apart. */
export function mapConnections(
  links: readonly AggregatedLink[],
  history: readonly HistoryLink[] | null,
  declared?: DeclaredTopology
): MapConnection[] {
  const current = neighborConnections(links)
  if (declared) {
    const byPair = new Map(
      current.map((connection) => [connection.key, connection])
    )
    const past = new Map(
      groupNodePairs(history ?? []).map((pair) => [pair.key, pair])
    )
    return declared.edges.map((edge) => {
      const connection = byPair.get(edge.key)
      const unavailable = edge.transports?.length === 0
      const historical = !connection ? past.get(edge.key) : undefined
      return {
        key: edge.key,
        a: edge.a,
        b: edge.b,
        declaration: edge,
        status: unavailable
          ? "unavailable"
          : (connection?.status ?? (historical ? "history" : "allowed")),
        look: unavailable
          ? "closed"
          : connection
            ? connectionLook(connection)
            : historical
              ? "history"
              : "allowed",
        transports: unavailable
          ? []
          : (connection?.transports ??
            (historical
              ? [
                  ...new Set(
                    historical.channels.map((channel) => channel.transport)
                  ),
                ]
              : [])),
      }
    })
  }
  const present = new Set(current.map((connection) => connection.key))
  return [
    ...current.map((connection) => ({
      key: connection.key,
      a: connection.a,
      b: connection.b,
      status: connection.status,
      transports: connection.transports,
      look: connectionLook(connection),
    })),
    ...groupNodePairs(history ?? [])
      .filter((pair) => !present.has(pair.key))
      .map((pair) => ({
        key: `history|${pair.key}`,
        a: pair.a,
        b: pair.b,
        status: "history" as const,
        look: "history" as const,
        transports: [
          ...new Set(pair.channels.map((channel) => channel.transport)),
        ].sort(),
      })),
  ]
}
