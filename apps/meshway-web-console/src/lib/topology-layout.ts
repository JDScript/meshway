import type { MapNode } from "@/lib/network-map"

// Where the topology puts its node cards: the whole network as a graph
// laid out by its links (started from, and turned to match, where each
// node is on Earth, so west stays left of east), or, from a chosen node,
// as columns along its paths. Both are in their own coordinates at full
// size; the canvas's view (zoom and pan) maps them to its pixels. They
// depend only on which nodes and links there are, and the paths: never on
// report times or transient state, so nothing jumps when a status changes.

export type Point = { x: number; y: number }
export type Box = { width: number; height: number }

/** A node in the whole network: one line, its name. */
export const GRAPH_CARD = { w: 136, h: 32 }
/** A node on a chosen node's paths: its name and its state. */
export const PATH_CARD = { w: 164, h: 46 }

const position = (node: MapNode) =>
  node.geo?.kind === "found" ? node.geo.geo.position : null

export type Bounds = {
  left: number
  top: number
  right: number
  bottom: number
}

export function bounds(points: Iterable<Point>, margin = 0): Bounds {
  let left = Infinity
  let top = Infinity
  let right = -Infinity
  let bottom = -Infinity
  for (const p of points) {
    left = Math.min(left, p.x)
    top = Math.min(top, p.y)
    right = Math.max(right, p.x)
    bottom = Math.max(bottom, p.y)
  }
  if (left === Infinity) return { left: 0, top: 0, right: 1, bottom: 1 }
  return {
    left: left - margin,
    top: top - margin,
    right: right + margin,
    bottom: bottom + margin,
  }
}

/** Kept clear around a card: from other cards, and from lines not its own. */
const CLEAR = { card: 18, line: 10 }
/** The closest two cards side by side can be, centre to centre (the canvas sizes cards by it too). */
export const GRAPH_PITCH = GRAPH_CARD.w + CLEAR.card

/**
 * Moves cards apart until none overlap and no line runs under a card that
 * is not one of its ends (in place, as far as a few passes go).
 */
function untangle(
  xs: Float64Array,
  ys: Float64Array,
  links: readonly (readonly [number, number])[]
) {
  const n = xs.length
  const w = GRAPH_CARD.w + CLEAR.card
  const h = GRAPH_CARD.h + CLEAR.card
  const hw = GRAPH_CARD.w / 2 + CLEAR.line
  const hh = GRAPH_CARD.h / 2 + CLEAR.line
  for (let pass = 0; pass < 160; pass++) {
    let moved = false
    for (let i = 0; i < n; i++)
      for (let j = i + 1; j < n; j++) {
        const dx = (xs[j] ?? 0) - (xs[i] ?? 0)
        const dy = (ys[j] ?? 0) - (ys[i] ?? 0)
        const ox = w - Math.abs(dx)
        const oy = h - Math.abs(dy)
        if (ox <= 0 || oy <= 0) continue
        moved = true
        // Out the cheaper way, half each.
        if (oy * w < ox * h) {
          const dir = dy > 0 || (dy === 0 && j % 2 === 0) ? 1 : -1
          ys[i] = (ys[i] ?? 0) - (dir * oy) / 2
          ys[j] = (ys[j] ?? 0) + (dir * oy) / 2
        } else {
          const dir = dx > 0 || (dx === 0 && j % 2 === 0) ? 1 : -1
          xs[i] = (xs[i] ?? 0) - (dir * ox) / 2
          xs[j] = (xs[j] ?? 0) + (dir * ox) / 2
        }
      }
    for (const [a, b] of links) {
      const ax = xs[a] ?? 0
      const ay = ys[a] ?? 0
      const lx = (xs[b] ?? 0) - ax
      const ly = (ys[b] ?? 0) - ay
      const length = Math.hypot(lx, ly)
      if (length < 1) continue
      const ux = lx / length
      const uy = ly / length
      for (let c = 0; c < n; c++) {
        if (c === a || c === b) continue
        const cx = (xs[c] ?? 0) - ax
        const cy = (ys[c] ?? 0) - ay
        const along = cx * ux + cy * uy
        // The card's reach along the line and across it.
        const reach = hw * Math.abs(ux) + hh * Math.abs(uy)
        if (along < -reach || along > length + reach) continue
        const across = cx * -uy + cy * ux
        const room = hw * Math.abs(uy) + hh * Math.abs(ux)
        if (Math.abs(across) >= room) continue
        // Only where the line is between its ends, not past them.
        if (along < 0 || along > length) continue
        moved = true
        const side = across >= 0 ? 1 : -1
        const push = room - Math.abs(across)
        // The card moves off the line; the line gives a little too.
        xs[c] = (xs[c] ?? 0) + -uy * side * push * 0.75
        ys[c] = (ys[c] ?? 0) + ux * side * push * 0.75
        const back = push * 0.125
        xs[a] = (xs[a] ?? 0) - -uy * side * back
        ys[a] = (ys[a] ?? 0) - ux * side * back
        xs[b] = (xs[b] ?? 0) - -uy * side * back
        ys[b] = (ys[b] ?? 0) - ux * side * back
      }
    }
    if (!moved) break
  }
}

/** Card centres apart along a link (before the layout is flattened). */
const LINK = 120
/** Cards are wider than tall: the layout is flattened by this much. */
const FLATTEN = 1

/**
 * The best turn (and mirror, if that fits better) of `points` about their
 * middle to match `towards`, both over the same indices; applied in place.
 */
function align(xs: Float64Array, ys: Float64Array, towards: (Point | null)[]) {
  const at = towards.flatMap((p, i) => (p ? [i] : []))
  if (at.length < 2) return
  const mean = (values: number[]) =>
    values.reduce((sum, v) => sum + v, 0) / values.length
  const cx = mean(at.map((i) => xs[i] ?? 0))
  const cy = mean(at.map((i) => ys[i] ?? 0))
  const tx = mean(at.map((i) => towards[i]?.x ?? 0))
  const ty = mean(at.map((i) => towards[i]?.y ?? 0))
  const fit = (mirror: number) => {
    let dot = 0
    let cross = 0
    for (const i of at) {
      const x = (xs[i] ?? 0) - cx
      const y = ((ys[i] ?? 0) - cy) * mirror
      const u = (towards[i]?.x ?? 0) - tx
      const v = (towards[i]?.y ?? 0) - ty
      dot += x * u + y * v
      cross += x * v - y * u
    }
    return { angle: Math.atan2(cross, dot), score: Math.hypot(dot, cross) }
  }
  const plain = fit(1)
  const mirrored = fit(-1)
  const mirror = mirrored.score > plain.score ? -1 : 1
  const { angle } = mirror === 1 ? plain : mirrored
  const cos = Math.cos(angle)
  const sin = Math.sin(angle)
  for (let i = 0; i < xs.length; i++) {
    const x = (xs[i] ?? 0) - cx
    const y = ((ys[i] ?? 0) - cy) * mirror
    xs[i] = x * cos - y * sin
    ys[i] = x * sin + y * cos
  }
}

/**
 * The whole network: a force layout of the cards over the links (springs
 * along links, every two cards pushed apart, all held to the middle),
 * started from where each node is on Earth and turned at the end to match
 * it as well as it can, so west stays left of east; flattened for wide
 * cards, then cards moved apart until none overlap. Always the same for
 * the same nodes and links.
 */
export function graphLayout(
  nodes: readonly MapNode[],
  pairs: readonly { a: string; b: string }[]
): Map<string, Point> {
  const ids = nodes.map((node) => node.id).sort()
  const n = ids.length
  if (n === 0) return new Map()
  const index = new Map(ids.map((id, i) => [id, i]))
  const byId = new Map(nodes.map((node) => [node.id, node]))
  // In one order, whatever order they come in: the same layout every time.
  const seen = new Set<number>()
  const links = pairs
    .flatMap(({ a, b }): [number, number][] => {
      const i = index.get(a)
      const j = index.get(b)
      if (i === undefined || j === undefined || i === j) return []
      return [i < j ? [i, j] : [j, i]]
    })
    .sort((x, y) => x[0] - y[0] || x[1] - y[1])
    .filter(([i, j]) => !seen.has(i * n + j) && seen.add(i * n + j))
  // Earth, flattened to about the size the layout takes.
  const radius = LINK * Math.sqrt(n) * 0.55
  const earth = ids.map((id): Point | null => {
    const node = byId.get(id)
    const at = node ? position(node) : null
    return at
      ? { x: (at.lng / 180) * radius, y: (-at.lat / 90) * radius * 0.6 }
      : null
  })
  const neighbours = ids.map(() => [] as number[])
  for (const [i, j] of links) {
    neighbours[i]?.push(j)
    neighbours[j]?.push(i)
  }
  const xs = new Float64Array(n)
  const ys = new Float64Array(n)
  ids.forEach((_, i) => {
    const home = earth[i]
    // Unplaced: beside its placed neighbours, or below the middle.
    const near = (neighbours[i] ?? []).flatMap((j) => earth[j] ?? [])
    const base =
      home ??
      (near.length
        ? {
            x: near.reduce((sum, p) => sum + p.x, 0) / near.length,
            y: near.reduce((sum, p) => sum + p.y, 0) / near.length,
          }
        : { x: 0, y: radius * 0.5 })
    // Nodes in one place start a little apart.
    const angle = i * 2.399963
    const spread = home ? LINK * 0.15 : LINK * 0.6
    xs[i] = base.x + Math.cos(angle) * spread
    ys[i] = base.y + Math.sin(angle) * spread
  })

  const fx = new Float64Array(n)
  const fy = new Float64Array(n)
  // Fewer rounds for large networks; every round compares every two.
  const rounds = Math.max(80, Math.min(400, Math.round(6e6 / (n * n))))
  let heat = LINK * 0.6
  for (let round = 0; round < rounds; round++) {
    fx.fill(0)
    fy.fill(0)
    for (let i = 0; i < n; i++)
      for (let j = i + 1; j < n; j++) {
        const dx = (xs[i] ?? 0) - (xs[j] ?? 0) || 0.01 * (j - i)
        const dy = (ys[i] ?? 0) - (ys[j] ?? 0) || 0.01
        const d2 = Math.max(1, dx * dx + dy * dy)
        const push = (LINK * LINK) / d2
        fx[i] = (fx[i] ?? 0) + dx * push
        fy[i] = (fy[i] ?? 0) + dy * push
        fx[j] = (fx[j] ?? 0) - dx * push
        fy[j] = (fy[j] ?? 0) - dy * push
      }
    for (const [i, j] of links) {
      const dx = (xs[i] ?? 0) - (xs[j] ?? 0)
      const dy = (ys[i] ?? 0) - (ys[j] ?? 0)
      // A node on one link only sits close to it.
      const leaf =
        Math.min(neighbours[i]?.length ?? 0, neighbours[j]?.length ?? 0) <= 1
      const pull = Math.hypot(dx, dy) / (leaf ? LINK * 0.7 : LINK)
      fx[i] = (fx[i] ?? 0) - dx * pull
      fy[i] = (fy[i] ?? 0) - dy * pull
      fx[j] = (fx[j] ?? 0) + dx * pull
      fy[j] = (fy[j] ?? 0) + dy * pull
    }
    for (let i = 0; i < n; i++) {
      const x = xs[i] ?? 0
      const y = ys[i] ?? 0
      const gx = (fx[i] ?? 0) - x * 0.35
      const gy = (fy[i] ?? 0) - y * 0.35
      const d = Math.max(1e-6, Math.hypot(gx, gy))
      const step = Math.min(d, heat)
      xs[i] = x + (gx / d) * step
      ys[i] = y + (gy / d) * step
    }
    heat = Math.max(1, heat * 0.985)
  }
  align(xs, ys, earth)
  for (let i = 0; i < n; i++) ys[i] = (ys[i] ?? 0) * FLATTEN
  untangle(xs, ys, links)
  return new Map(ids.map((id, i) => [id, { x: xs[i] ?? 0, y: ys[i] ?? 0 }]))
}

/** Card centres one above another in a column (the canvas sizes cards by it too). */
export const PATH_ROW = PATH_CARD.h + 16
/** Between the parts reached through different nodes. */
const BRANCH_GAP = 18
const COLUMN = PATH_CARD.w + 150

/** Each node's hop count from the source, and the node it is reached through. */
export type RouteTree = {
  depth: ReadonlyMap<string, number>
  parent: ReadonlyMap<string, string>
}

type Contour = Map<number, { lo: number; hi: number }>

/**
 * From a chosen node, as a tree its paths fan out along: each node in the
 * column after the node it is reached through, its own next hops to its
 * right and centred on it, so lines never cross; the parts reached through
 * different nodes are packed as close as their columns allow (a contour,
 * as in a tidy tree). A node whose way in is not on the tree hangs from
 * the source; nodes with no path take a last column. `order` is the order
 * within a column.
 */
export function pathLayout(
  source: string,
  order: readonly string[],
  tree: RouteTree
): Map<string, Point> {
  const rank = new Map(order.map((id, index) => [id, index]))
  const byRank = (x: string, y: string) =>
    (rank.get(x) ?? 0) - (rank.get(y) ?? 0)
  const below = new Map<string, string[]>()
  for (const [id, parent] of tree.parent)
    if (id !== source && tree.depth.has(id))
      below.set(parent, [...(below.get(parent) ?? []), id])
  const column = new Map<string, number>([[source, 0]])
  const children = new Map<string, string[]>()
  const hang = (parent: string, id: string) => {
    column.set(id, (column.get(parent) ?? 0) + 1)
    children.set(parent, [...(children.get(parent) ?? []), id])
    for (const kid of [...(below.get(id) ?? [])].sort(byRank))
      if (!column.has(kid)) hang(id, kid)
  }
  for (const kid of [...(below.get(source) ?? [])].sort(byRank))
    if (!column.has(kid)) hang(source, kid)
  // Reached through a node with no path of its own (or round in a loop).
  const astray = [...tree.depth.keys()]
    .filter((id) => id !== source && !column.has(id))
    .sort(
      (x, y) =>
        (tree.depth.get(x) ?? 0) - (tree.depth.get(y) ?? 0) || byRank(x, y)
    )
  for (const id of astray) if (!column.has(id)) hang(source, id)

  /** Where each node below `id` sits relative to it, and the extent at each depth below. */
  const subtree = (
    id: string
  ): { ys: Map<string, number>; contour: Contour } => {
    const kids = children.get(id) ?? []
    // End nodes first, then the parts that go on: the next column of a
    // part that goes on can then rise beside them.
    const ordered = [
      ...kids.filter((kid) => (children.get(kid) ?? []).length === 0),
      ...kids.filter((kid) => (children.get(kid) ?? []).length > 0),
    ]
    const placed = new Map<string, number>()
    const contour: Contour = new Map()
    const heads: number[] = []
    ordered.forEach((kid, index) => {
      const inner = subtree(kid)
      const part: Contour = new Map([[1, { lo: 0, hi: 0 }]])
      for (const [at, extent] of inner.contour) part.set(at + 1, extent)
      const branch = (children.get(kid) ?? []).length > 0
      const before = ordered[index - 1]
      const gap =
        PATH_ROW +
        (branch || (before && (children.get(before) ?? []).length > 0)
          ? BRANCH_GAP
          : 0)
      let offset = 0
      if (index > 0) {
        offset = -Infinity
        for (const [at, extent] of part) {
          const above = contour.get(at)
          if (above) offset = Math.max(offset, above.hi + gap - extent.lo)
        }
        if (offset === -Infinity) offset = 0
      }
      placed.set(kid, offset)
      for (const [node, y] of inner.ys) placed.set(node, y + offset)
      for (const [at, extent] of part) {
        const was = contour.get(at)
        contour.set(at, {
          lo: Math.min(was?.lo ?? Infinity, extent.lo + offset),
          hi: Math.max(was?.hi ?? -Infinity, extent.hi + offset),
        })
      }
      heads.push(offset)
    })
    const centre = heads.length
      ? ((heads[0] ?? 0) + (heads.at(-1) ?? 0)) / 2
      : 0
    const ys = new Map<string, number>()
    for (const [node, y] of placed) ys.set(node, y - centre)
    const shifted: Contour = new Map([[0, { lo: 0, hi: 0 }]])
    for (const [at, extent] of contour)
      shifted.set(at, { lo: extent.lo - centre, hi: extent.hi - centre })
    return { ys, contour: shifted }
  }

  const placed = subtree(source)
  const points = new Map<string, Point>([[source, { x: 0, y: 0 }]])
  for (const [id, y] of placed.ys)
    points.set(id, { x: (column.get(id) ?? 0) * COLUMN, y })
  const unreached = order.filter((id) => !points.has(id))
  if (unreached.length) {
    const right = Math.max(...[...points.values()].map((p) => p.x))
    const height = (unreached.length - 1) * PATH_ROW
    unreached.forEach((id, index) =>
      points.set(id, { x: right + COLUMN, y: index * PATH_ROW - height / 2 })
    )
  }
  return points
}
