import type {
  AggregatedLink,
  Freshness,
  GeoObservation,
  LinkEvidence,
  LinkPhase,
  LocationAbsentReason,
  RecentLink,
  StateView,
} from "@meshway/control-client"

import type { LinkLook, Scene, SceneLink, SceneNode } from "../globe/scene"

// The overview's reading of the network's State: which nodes the
// controller located, which it did not and why, and which links can be
// drawn between located ends. Nothing here is estimated: a node without the
// controller's position is unlocated, a link with an unlocated end is
// counted, never drawn at an invented place.

/**
 * Why a node is not on the globe: the controller's `locationAbsent`, or
 * `noPosition` when its GeoIP result names a place but no coordinates.
 */
export type Unlocated = LocationAbsentReason | "noPosition" | "unknown"

export type MapNode = {
  id: string
  name: string
  freshness: Freshness
  /** The controller's derived location; set even without coordinates. */
  location: GeoObservation | null
  /** Set exactly when the node is not on the globe. */
  unlocated: Unlocated | null
  receivedAt: string | null
  lastAuthenticatedAt: string | null
}

/** Freshness in the order the overview lists it. */
export const FRESHNESS_ORDER: readonly Freshness[] = [
  "current",
  "awaiting_report",
  "expired",
  "config_mismatch",
  "superseded",
  "revoked",
]

export const PHASE_ORDER: readonly LinkPhase[] = [
  "authenticated",
  "preparing",
  "failed",
  "closed",
]

export const EVIDENCE_ORDER: readonly LinkEvidence[] = [
  "matched",
  "oneSided",
  "conflicting",
  "expired",
]

/** The first 8 characters of an ID, for a node the inventory does not name to this account. */
export const shortId = (id: string) => id.slice(0, 8)

/**
 * Every node in the State, named by the inventory (`names`); a node the
 * account cannot list (a peer of its own device) keeps its short ID.
 */
export function mapNodes(
  state: StateView,
  names: ReadonlyMap<string, string>
): MapNode[] {
  return Object.entries(state.nodes)
    .map(([id, view]): MapNode => {
      const location = view.derived.location ?? null
      const placed = location?.data.position != null
      return {
        id,
        name: names.get(id) ?? shortId(id),
        freshness: view.freshness,
        location,
        unlocated: placed
          ? null
          : location
            ? "noPosition"
            : (view.derived.locationAbsent ?? "unknown"),
        receivedAt: view.receivedAt ?? null,
        lastAuthenticatedAt: view.lastAuthenticatedAt ?? null,
      }
    })
    .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
}

export function sceneNode(node: MapNode): SceneNode {
  const position = node.location?.data.position
  return {
    id: node.id,
    name: node.name,
    placement: position
      ? {
          lat: position.latitude,
          lng: position.longitude,
          accuracyKm: position.accuracyRadiusKm ?? null,
        }
      : null,
    current: node.freshness === "current",
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
export function partitionLinks<T extends { a: string; b: string }>(
  links: readonly T[],
  located: ReadonlySet<string>
): { drawn: T[]; undrawn: T[] } {
  const drawn: T[] = []
  const undrawn: T[] = []
  for (const link of links)
    (located.has(link.a) && located.has(link.b) ? drawn : undrawn).push(link)
  return { drawn, undrawn }
}

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
  const current = partitionLinks(links, located).drawn
  const currentKeys = new Set(links.map(linkKey))
  const past = history
    ? partitionLinks(
        history.filter((link) => !currentKeys.has(linkKey(link))),
        located
      ).drawn
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
