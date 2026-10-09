import assert from "node:assert/strict"
import { test } from "vitest"

import type { MapNode } from "@/lib/network-map"
import {
  GRAPH_CARD,
  graphLayout,
  PATH_CARD,
  pathLayout,
  type Point,
} from "@/lib/topology-layout"

const idOf = (index: number) =>
  `00000000-0000-4000-8000-${String(index).padStart(12, "0")}`

const mapNode = (
  id: string,
  at: { lat: number; lng: number } | null
): MapNode => ({
  id,
  name: id,
  status: "current",
  location: null,
  geo: at
    ? {
        kind: "found",
        geo: {
          country: null,
          region: null,
          city: null,
          position: { ...at, accuracyKm: null },
          asn: null,
        },
      }
    : null,
  unlocated: at ? null : "unknown",
  receivedAt: null,
  lastAuthenticatedAt: null,
})

/** Four hubs around the world, each with six nodes near it, a few unplaced, hubs in a ring. */
function network() {
  const hubs = [
    { lat: 37, lng: -122 },
    { lat: 50, lng: 8 },
    { lat: 1, lng: 104 },
    { lat: 40, lng: -74 },
  ]
  const nodes: MapNode[] = []
  const pairs: { a: string; b: string }[] = []
  hubs.forEach((hub, h) => {
    nodes.push(mapNode(idOf(h), hub))
    pairs.push({ a: idOf(h), b: idOf((h + 1) % hubs.length) })
    for (let k = 0; k < 6; k++) {
      const id = idOf(10 + h * 10 + k)
      nodes.push(
        mapNode(id, { lat: hub.lat + (k % 3), lng: hub.lng + (k % 2) * 2 })
      )
      pairs.push({ a: idOf(h), b: id })
    }
  })
  for (let k = 0; k < 3; k++) {
    nodes.push(mapNode(idOf(90 + k), null))
    pairs.push({ a: idOf(0), b: idOf(90 + k) })
  }
  return { nodes, pairs }
}

const apart = (
  points: ReadonlyMap<string, Point>,
  card: { w: number; h: number }
) => {
  const all = [...points.values()]
  for (let i = 0; i < all.length; i++)
    for (let j = i + 1; j < all.length; j++) {
      const a = all[i]
      const b = all[j]
      assert.ok(a && b)
      assert.ok(
        Math.abs(a.x - b.x) >= card.w || Math.abs(a.y - b.y) >= card.h,
        "two cards overlap"
      )
    }
}

test("the whole network's layout is the same for the same nodes and links, in any order", () => {
  const { nodes, pairs } = network()
  const first = graphLayout(nodes, pairs)
  assert.equal(first.size, nodes.length)
  assert.deepEqual(
    first,
    graphLayout([...nodes].reverse(), [...pairs].reverse())
  )
  apart(first, GRAPH_CARD)
})

test("the whole network keeps west left of east", () => {
  const { nodes, pairs } = network()
  const points = graphLayout(nodes, pairs)
  const x = (id: string) => points.get(id)?.x ?? Number.NaN
  // San Francisco, New York, Frankfurt, Singapore.
  assert.ok(x(idOf(0)) < x(idOf(1)))
  assert.ok(x(idOf(3)) < x(idOf(1)))
  assert.ok(x(idOf(1)) < x(idOf(2)))
})

test("a node's paths go out in columns, each node right after the one it is reached through", () => {
  const [S, R1, R2, A, B, C, D, LOST] = [1, 2, 3, 4, 5, 6, 7, 8].map(idOf)
  assert.ok(S && R1 && R2 && A && B && C && D && LOST)
  const points = pathLayout(S, [S, R1, R2, A, B, C, D, LOST], {
    depth: new Map([
      [R1, 1],
      [R2, 1],
      [A, 2],
      [B, 2],
      [C, 2],
      // Reached through a node with no path of its own.
      [D, 3],
    ]),
    parent: new Map([
      [R1, S],
      [R2, S],
      [A, R1],
      [B, R1],
      [C, R2],
      [D, LOST],
    ]),
  })
  const at = (id: string) => {
    const point = points.get(id)
    assert.ok(point)
    return point
  }
  assert.deepEqual(at(S), { x: 0, y: 0 })
  assert.equal(at(R1).x, at(R2).x)
  assert.ok(at(A).x > at(R1).x && at(A).x === at(C).x)
  // Hangs from the source; the node with no path is in a last column.
  assert.equal(at(D).x, at(R1).x)
  assert.ok(at(LOST).x > at(A).x)
  // A node's next hops are around it, so lines from different nodes do not cross.
  assert.ok(
    Math.max(at(A).y, at(B).y) < at(C).y || Math.min(at(A).y, at(B).y) > at(C).y
  )
  apart(points, PATH_CARD)
})
