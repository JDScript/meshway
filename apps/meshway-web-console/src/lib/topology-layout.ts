// Where the topology draws its nodes, in the canvas's own pixels. Positions
// depend only on the order of the ids (callers sort them by place, so the
// diagram reads roughly like the globe), the box, and in the focused view
// the tree of planned paths: never on report times or transient state, so
// nothing jumps when a status changes.

export type Point = { x: number; y: number }
export type Box = { width: number; height: number }

/** Room either side of a dot for the name under it, and below the lowest. */
const SIDE_ROOM = 72
const BOTTOM_ROOM = 44
/** The least distance along a ring between two dots. */
const SPACING = 104

type Frame = { cx: number; cy: number; rx: number; ry: number }

function frameOf(box: Box): Frame {
  // A narrow box (a phone) keeps less room around the rings.
  const side = Math.min(SIDE_ROOM, box.width * 0.16)
  const left = side
  const right = Math.max(left + 1, box.width - side)
  const top = 24
  const bottom = Math.max(top + 1, box.height - BOTTOM_ROOM)
  return {
    cx: (left + right) / 2,
    cy: (top + bottom) / 2,
    rx: (right - left) / 2,
    ry: (bottom - top) / 2,
  }
}

const at = (frame: Frame, scale: number, angle: number): Point => ({
  x: frame.cx + Math.cos(angle) * frame.rx * scale,
  y: frame.cy + Math.sin(angle) * frame.ry * scale,
})

const meanRadius = (frame: Frame) =>
  Math.sqrt((frame.rx ** 2 + frame.ry ** 2) / 2)

/** How many dots a ring of `scale` holds at `SPACING`. */
function capacity(frame: Frame, scale: number) {
  const perimeter = 2 * Math.PI * meanRadius(frame) * scale
  return Math.max(3, Math.floor(perimeter / SPACING))
}

/** A few nodes stay close together: the ring only grows as they need room. */
const ARC_PER_NODE = 210
const SMALLEST_RING = 170
function fitted(frame: Frame, nodes: number) {
  const wanted = Math.max(SMALLEST_RING, (nodes * ARC_PER_NODE) / (2 * Math.PI))
  return Math.min(1, wanted / meanRadius(frame))
}

const TOP = -Math.PI / 2

/** Every node on rings, the outer ring first; one node in the middle, two side by side. */
export function overviewLayout(
  ids: readonly string[],
  box: Box
): Map<string, Point> {
  const frame = frameOf(box)
  const result = new Map<string, Point>()
  if (ids.length === 1 && ids[0] !== undefined)
    result.set(ids[0], { x: frame.cx, y: frame.cy })
  else if (ids.length === 2)
    ids.forEach((id, index) =>
      result.set(id, at(frame, fitted(frame, 2), index === 0 ? Math.PI : 0))
    )
  else if (ids.length <= capacity(frame, 1)) {
    const scale = fitted(frame, ids.length)
    ids.forEach((id, index) =>
      result.set(id, at(frame, scale, TOP + (index * 2 * Math.PI) / ids.length))
    )
  } else {
    let rest = [...ids]
    for (const scale of [1, 0.64, 0.32]) {
      if (rest.length === 0) break
      const ring = scale === 0.32 ? rest : rest.slice(0, capacity(frame, scale))
      rest = rest.slice(ring.length)
      ring.forEach((id, index) =>
        result.set(
          id,
          at(frame, scale, TOP + (index * 2 * Math.PI) / ring.length)
        )
      )
    }
  }
  return result
}

export type RouteTree = {
  depth: ReadonlyMap<string, number>
  parent: ReadonlyMap<string, string>
}

/**
 * The source in the middle; each node on the ring of its hop count, near
 * the node it is reached through; nodes without a planned path on the
 * outermost ring.
 */
export function focusLayout(
  source: string,
  ids: readonly string[],
  tree: RouteTree,
  box: Box
): Map<string, Point> {
  const frame = frameOf(box)
  const result = new Map<string, Point>([
    [source, { x: frame.cx, y: frame.cy }],
  ])
  const others = ids.filter((id) => id !== source)
  const reached = others.filter((id) => tree.depth.has(id))
  const unreached = others.filter((id) => !tree.depth.has(id))
  const deepest = Math.max(0, ...reached.map((id) => tree.depth.get(id) ?? 0))
  const rings = deepest + (unreached.length > 0 ? 1 : 0)
  if (rings === 0) return result
  const outer = Math.max(
    fitted(frame, others.length),
    Math.min(1, rings * 0.34)
  )
  const scaleOf = (ring: number) => outer * (0.2 + (0.8 * ring) / rings)

  // Each placed node owns a sector; its children share it.
  const sectors = new Map<string, { from: number; span: number }>([
    [source, { from: TOP, span: 2 * Math.PI }],
  ])
  for (let ring = 1; ring <= deepest; ring++) {
    const onRing = reached.filter((id) => tree.depth.get(id) === ring)
    const byParent = new Map<string, string[]>()
    for (const id of onRing) {
      const parent = tree.parent.get(id)
      const key = parent !== undefined && sectors.has(parent) ? parent : source
      byParent.set(key, [...(byParent.get(key) ?? []), id])
    }
    for (const [parent, children] of byParent) {
      const sector = sectors.get(parent) ?? { from: TOP, span: 2 * Math.PI }
      const step = sector.span / children.length
      children.forEach((id, index) => {
        // Around the source one or two nodes sit beside it (their names
        // read to the right), more start at the top; a parent's children
        // share its sector.
        const angle =
          parent === source
            ? (children.length <= 2 ? 0 : TOP) + index * step
            : sector.from + (index + 0.5) * step
        result.set(id, at(frame, scaleOf(ring), angle))
        sectors.set(id, { from: angle - step / 2, span: step })
      })
    }
  }
  unreached.forEach((id, index) =>
    result.set(
      id,
      at(
        frame,
        scaleOf(rings),
        TOP + ((index + 0.5) * 2 * Math.PI) / unreached.length
      )
    )
  )
  return result
}
