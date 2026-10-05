// What the globe shows, independent of how it is rendered.
//
// Position and connection evidence stay strictly apart:
// - a node is on the globe only with the controller's derived location (a
//   GeoIP result for an underlay IP the node observed); every other node is
//   left off the globe and listed as unlocated. Names, Mesh IPs and an
//   administrator's country are never locations.
// - links are the channels the nodes' current State reports, styled by
//   their reported phase and the controller's evidence; history links come
//   from recorded link events and are drawn apart. Grants, allowed
//   topology and planned paths are not links. Nothing moves along a link:
//   no traffic is measured here.

import type { NodeStatus } from "@/lib/nodes/status"

/** Where the controller's GeoIP places a node; approximate, never GPS. */
export type GeoPlacement = {
  lat: number
  lng: number
  /** The dataset's estimate; null when it gives none. */
  accuracyKm: number | null
}

export type SceneNode = {
  id: string
  name: string
  placement: GeoPlacement | null
  /** Its status (`lib/nodes/status.ts`), drawn in its tone's colour. */
  status: NodeStatus
}

/**
 * - `matched`: authenticated, both ends report the same channel.
 * - `one_sided`: authenticated, one end reports it.
 * - `conflicting`: the two ends' reports disagree.
 * - `expired`: a report it rests on is no longer current.
 * - `preparing` / `failed` / `closed`: the reported phase.
 * - `history`: seen in the chosen history window, not current.
 */
export type LinkLook =
  | "matched"
  | "one_sided"
  | "conflicting"
  | "expired"
  | "preparing"
  | "failed"
  | "closed"
  | "history"

export type SceneLink = {
  key: string
  nodeIds: [string, string]
  look: LinkLook
  /** Among links between the same two nodes: 0, 1, 2… (each arcs a little higher). */
  lane: number
}

export type Scene = {
  nodes: SceneNode[]
  links: SceneLink[]
}

export const EMPTY_SCENE: Scene = { nodes: [], links: [] }

export type Anchor = { lat: number; lng: number; altitude: number }

/** Globe radius in world units (three-globe's), shared by the stage and every layer. */
export const GLOBE_RADIUS = 100

export type Vec3 = [number, number, number]

const rad = (degrees: number) => (degrees * Math.PI) / 180

export function cartesian(lat: number, lng: number, altitude = 0): Vec3 {
  const phi = rad(90 - lat)
  const theta = rad(90 - lng)
  const r = GLOBE_RADIUS * (1 + altitude)
  return [
    r * Math.sin(phi) * Math.cos(theta),
    r * Math.cos(phi),
    r * Math.sin(phi) * Math.sin(theta),
  ]
}

/** Where each placed node sits; unplaced nodes have no anchor and are not drawn. */
export function anchorsOf(nodes: readonly SceneNode[]): Map<string, Anchor> {
  const anchors = new Map<string, Anchor>()
  for (const node of nodes) {
    if (node.placement)
      anchors.set(node.id, {
        lat: node.placement.lat,
        lng: node.placement.lng,
        altitude: 0.01,
      })
  }
  return anchors
}

const length = ([x, y, z]: Vec3) => Math.hypot(x, y, z)
const unit = (v: Vec3): Vec3 => {
  const l = length(v) || 1
  return [v[0] / l, v[1] / l, v[2] / l]
}

const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
const cross = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
]

/** The angle (radians) between two anchors, seen from the globe's centre. */
export function angleBetween(a: Anchor, b: Anchor): number {
  const da = cartesian(a.lat, a.lng)
  const db = cartesian(b.lat, b.lng)
  const cos = dot(da, db) / (GLOBE_RADIUS * GLOBE_RADIUS)
  return Math.acos(Math.min(1, Math.max(-1, cos)))
}

/**
 * Nodes too close to tell apart, as groups: two placed nodes `near` each
 * other are in one group, and so is any node near one of a group's. Groups
 * and their members follow `order`; a node alone is a group of one.
 * Nothing is moved: a group is drawn at its first member.
 */
export function groupNearby(
  order: readonly string[],
  anchors: ReadonlyMap<string, Anchor>,
  near: (a: Anchor, b: Anchor) => boolean
): string[][] {
  const placed = order.filter((id) => anchors.has(id))
  const groupOf = new Map<string, number>()
  const groups: string[][] = []
  for (const id of placed) {
    const anchor = anchors.get(id)
    if (!anchor) continue
    const close = new Set<number>()
    for (const [other, index] of groupOf) {
      const at = anchors.get(other)
      if (at && near(anchor, at)) close.add(index)
    }
    const [first, ...rest] = [...close].sort((x, y) => x - y)
    if (first === undefined) {
      groupOf.set(id, groups.length)
      groups.push([id])
      continue
    }
    // Joining several groups merges them into the earliest.
    for (const index of rest) {
      const members = groups[index] ?? []
      for (const member of members) groupOf.set(member, first)
      groups[first]?.push(...members)
      groups[index] = []
    }
    groupOf.set(id, first)
    groups[first]?.push(id)
  }
  const rank = new Map(placed.map((id, index) => [id, index]))
  return groups
    .filter((group) => group.length > 0)
    .map((group) =>
      group.sort((x, y) => (rank.get(x) ?? 0) - (rank.get(y) ?? 0))
    )
}

/**
 * Where `b` is on screen relative to `a` (CSS px, right and down) at
 * `perRadian` CSS px per radian of the globe facing the camera. North is up
 * near the middle of the view; towards the edge this is an estimate.
 */
export function screenOffset(
  a: Anchor,
  b: Anchor,
  perRadian: number
): [number, number] {
  const rad = Math.PI / 180
  const across = ((((b.lng - a.lng + 180) % 360) + 360) % 360) - 180
  return [
    across * rad * Math.cos(((a.lat + b.lat) / 2) * rad) * perRadian,
    (a.lat - b.lat) * rad * perRadian,
  ]
}

/** A marker as laid out around its dot (CSS px): the dot's radius, and its name from `left` for `width`, `height` tall. */
export type MarkerBox = {
  key: string
  anchor: Anchor
  dot: number
  left: number
  width: number
  height: number
}

/**
 * The markers whose names fit: taken in order, a name is shown unless it
 * would cover another marker's dot or a name already shown. Dots are
 * always shown.
 */
export function fittingLabels(
  boxes: readonly MarkerBox[],
  perRadian: number
): Set<string> {
  type Rect = readonly [number, number, number, number]
  const overlaps = (a: Rect, b: Rect) =>
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
  const label = (box: MarkerBox, dx = 0, dy = 0): Rect => [
    dx + box.left,
    dy - box.height / 2,
    dx + box.left + box.width,
    dy + box.height / 2,
  ]
  const shown = new Set<MarkerBox>()
  for (const box of boxes) {
    const own = label(box)
    const covers = boxes.some((other) => {
      if (other === box) return false
      const [dx, dy] = screenOffset(box.anchor, other.anchor, perRadian)
      const dot: Rect = [
        dx - other.dot,
        dy - other.dot,
        dx + other.dot,
        dy + other.dot,
      ]
      return (
        overlaps(own, dot) ||
        (shown.has(other) && overlaps(own, label(other, dx, dy)))
      )
    })
    if (!covers) shown.add(box)
  }
  return new Set([...shown].map((box) => box.key))
}

/**
 * A link's path: the great circle between its ends, lifted into an arc whose
 * height grows with distance, so it never dips into the globe. Opposite ends
 * have no single great circle; the path then runs over the pole side, so it
 * is still one smooth arc rather than an undefined one.
 */
export function linkCurve(
  a: Anchor,
  b: Anchor,
  samples = 72,
  lane = 0
): Vec3[] {
  const da = unit(cartesian(a.lat, a.lng))
  const db = unit(cartesian(b.lat, b.lng))
  const omega = Math.acos(Math.min(1, Math.max(-1, dot(da, db))))
  // The unit vector in the arc's plane, perpendicular to `da`, towards `db`
  // (or, for opposite ends, towards the north pole).
  const pole: Vec3 = Math.abs(da[1]) > 0.99 ? [1, 0, 0] : [0, 1, 0]
  const towards = unit(cross(cross(da, Math.PI - omega < 1e-3 ? pole : db), da))
  const bulge = (0.03 + 0.34 * (omega / Math.PI)) * (1 + 0.22 * lane)
  const points: Vec3[] = []
  for (let i = 0; i <= samples; i += 1) {
    const t = i / samples
    const angle = t * omega
    const direction: Vec3 =
      i === samples
        ? db
        : omega < 1e-6
          ? da
          : unit([
              da[0] * Math.cos(angle) + towards[0] * Math.sin(angle),
              da[1] * Math.cos(angle) + towards[1] * Math.sin(angle),
              da[2] * Math.cos(angle) + towards[2] * Math.sin(angle),
            ])
    const altitude =
      a.altitude + (b.altitude - a.altitude) * t + bulge * Math.sin(Math.PI * t)
    const r = GLOBE_RADIUS * (1 + altitude)
    points.push([direction[0] * r, direction[1] * r, direction[2] * r])
  }
  return points
}

type LineStyle = {
  dashed: boolean
  dash: number
  gap: number
  opacity: number
}

/** A steady line for every link; its colour and pattern tell the evidence apart. */
export function lineStyle(look: LinkLook): LineStyle {
  switch (look) {
    case "matched":
      return { dashed: false, dash: 0, gap: 0, opacity: 0.8 }
    case "one_sided":
      return { dashed: false, dash: 0, gap: 0, opacity: 0.7 }
    case "conflicting":
      return { dashed: true, dash: 2.4, gap: 1.6, opacity: 0.85 }
    case "failed":
      return { dashed: true, dash: 1.2, gap: 1.4, opacity: 0.8 }
    case "preparing":
      return { dashed: true, dash: 0.7, gap: 1.8, opacity: 0.7 }
    case "expired":
    case "closed":
      return { dashed: true, dash: 0.5, gap: 2.6, opacity: 0.55 }
    case "history":
      return { dashed: false, dash: 0, gap: 0, opacity: 0.22 }
  }
}
