// What the globe shows, independent of how it is rendered.
//
// Position and connection evidence stay strictly apart:
// - a node is on the globe only with a placement that names its source and
//   observation time (no such source exists yet); every other node is left
//   off the globe and listed as unplaced. Names, Mesh IPs and the legacy
//   administrator country are never locations.
// - links are configured channels with the agents' own fresh reports
//   (features/topology). Grants and planned paths are not links.

/** Region-level at best, always with provenance. */
export type GeoPlacement = {
  lat: number
  lng: number
  /** e.g. "controller GeoIP of the STUN-observed underlay IP". */
  source: string
  observedAt: string
}

/**
 * - `live`: the agent's runtime report is current (it breathes).
 * - `contact`: recent control-plane contact only; not data-plane evidence.
 * - `quiet`: neither.
 */
export type Vitality = "live" | "contact" | "quiet"

export type SceneNode = {
  id: string
  name: string
  placement: GeoPlacement | null
  vitality: Vitality
}

/** Only `confirmed` flows: both ends report a fresh authenticated probe. */
export type LinkLook = "confirmed" | "one_sided" | "degraded" | "configured"

export type SceneLink = {
  key: string
  nodeIds: [string, string]
  look: LinkLook
}

export type Scene = {
  nodes: SceneNode[]
  links: SceneLink[]
}

export const EMPTY_SCENE: Scene = { nodes: [], links: [] }

export type Anchor = { lat: number; lng: number; altitude: number }

/** Same convention as three-globe, so custom layers line up with its globe. */
export const GLOBE_RADIUS = 100

export type Vec3 = [number, number, number]

const rad = (degrees: number) => (degrees * Math.PI) / 180

export function cartesian(lat: number, lng: number, altitude = 0): Vec3 {
  const phi = rad(90 - lat)
  const theta = rad(90 - lng)
  const r = GLOBE_RADIUS * (1 + altitude)
  return [r * Math.sin(phi) * Math.cos(theta), r * Math.cos(phi), r * Math.sin(phi) * Math.sin(theta)]
}

/** Where each placed node sits; unplaced nodes have no anchor and are not drawn. */
export function anchorsOf(nodes: readonly SceneNode[]): Map<string, Anchor> {
  const anchors = new Map<string, Anchor>()
  for (const node of nodes) {
    if (node.placement) anchors.set(node.id, { lat: node.placement.lat, lng: node.placement.lng, altitude: 0.01 })
  }
  return anchors
}

const length = ([x, y, z]: Vec3) => Math.hypot(x, y, z)
const unit = (v: Vec3): Vec3 => {
  const l = length(v) || 1
  return [v[0] / l, v[1] / l, v[2] / l]
}

const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
const cross = (a: Vec3, b: Vec3): Vec3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]

/**
 * A link's path: the great circle between its ends, lifted into an arc whose
 * height grows with distance, so it never dips into the globe. Opposite ends
 * have no single great circle; the path then runs over the pole side, so it
 * is still one smooth arc rather than an undefined one.
 */
export function linkCurve(a: Anchor, b: Anchor, samples = 72): Vec3[] {
  const da = unit(cartesian(a.lat, a.lng))
  const db = unit(cartesian(b.lat, b.lng))
  const omega = Math.acos(Math.min(1, Math.max(-1, dot(da, db))))
  // The unit vector in the arc's plane, perpendicular to `da`, towards `db`
  // (or, for opposite ends, towards the north pole).
  const pole: Vec3 = Math.abs(da[1]) > 0.99 ? [1, 0, 0] : [0, 1, 0]
  const towards = unit(cross(cross(da, Math.PI - omega < 1e-3 ? pole : db), da))
  const bulge = 0.03 + 0.34 * (omega / Math.PI)
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
    const altitude = a.altitude + (b.altitude - a.altitude) * t + bulge * Math.sin(Math.PI * t)
    const r = GLOBE_RADIUS * (1 + altitude)
    points.push([direction[0] * r, direction[1] * r, direction[2] * r])
  }
  return points
}

export type LineStyle = { dashed: boolean; dash: number; gap: number; opacity: number }

/** The steady line every link has; only its pattern tells the evidence apart. */
export function lineStyle(look: LinkLook): LineStyle {
  switch (look) {
    case "confirmed":
      return { dashed: false, dash: 0, gap: 0, opacity: 0.55 }
    case "one_sided":
      return { dashed: false, dash: 0, gap: 0, opacity: 0.75 }
    case "degraded":
      return { dashed: true, dash: 2.4, gap: 1.6, opacity: 0.85 }
    case "configured":
      return { dashed: true, dash: 0.7, gap: 1.8, opacity: 0.7 }
  }
}

/**
 * The light pulses on a confirmed link: one stream each way at a steady
 * pace, because the dialling direction is not the traffic direction and no
 * traffic is measured. Nothing flows without fresh two-sided evidence or
 * when reduced motion is requested.
 */
export function pulseStreams(look: LinkLook, motion: boolean): { reverse: boolean; phase: number }[] {
  if (look !== "confirmed" || !motion) return []
  return [
    { reverse: false, phase: 0 },
    { reverse: true, phase: 0.5 },
  ]
}

/** Arc length of a sampled curve, in globe units. */
export function curveLength(points: readonly Vec3[]): number {
  let total = 0
  for (let i = 1; i < points.length; i += 1) {
    const a = points[i - 1]
    const b = points[i]
    if (a && b) total += Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2])
  }
  return total
}
