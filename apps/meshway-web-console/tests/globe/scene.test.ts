import assert from "node:assert/strict"
import { test } from "vitest"

import {
  anchorsOf,
  angleBetween,
  cartesian,
  fittingLabels,
  GLOBE_RADIUS,
  groupNearby,
  lineStyle,
  linkCurve,
  screenOffset,
  type Anchor,
  type SceneNode,
} from "@/globe/scene"

const node = (
  id: string,
  name: string,
  placement: SceneNode["placement"] = null
): SceneNode => ({
  id,
  name,
  placement,
  status: "unknown",
})

const close = (p: readonly number[], q: readonly number[]) =>
  p.every((value, i) => Math.abs(value - (q[i] ?? 0)) < 1e-6)

test("only a sourced placement puts a node on the globe; unplaced nodes get no anchor", () => {
  const placed = node("p", "tokyo", { lat: 35.7, lng: 139.7, accuracyKm: 50 })
  const anchors = anchorsOf([placed, node("u", "unknown")])
  assert.deepEqual(anchors.get("p"), { lat: 35.7, lng: 139.7, altitude: 0.01 })
  assert.equal(anchors.has("u"), false)
})

function assertArc(a: Anchor, b: Anchor) {
  const curve = linkCurve(a, b, 32)
  const first = curve[0]
  const last = curve.at(-1)
  assert.ok(first && last)
  assert.ok(close(first, cartesian(a.lat, a.lng, a.altitude)))
  assert.ok(close(last, cartesian(b.lat, b.lng, b.altitude)))
  for (const point of curve) {
    assert.ok(point.every(Number.isFinite))
    assert.ok(
      Math.hypot(...point) >=
        GLOBE_RADIUS * (1 + Math.min(a.altitude, b.altitude)) - 1e-6
    )
  }
  // Consecutive samples stay close: one smooth arc, no jumps.
  const longest = Math.max(
    ...curve
      .slice(1)
      .map((point, i) =>
        Math.hypot(...point.map((value, k) => value - (curve[i]?.[k] ?? 0)))
      )
  )
  assert.ok(longest < GLOBE_RADIUS * 0.25, `step of ${longest}`)
}

test("a link curve joins its two anchors and stays above the surface", () => {
  assertArc(
    { lat: 10, lng: -20, altitude: 0.01 },
    { lat: 40, lng: 100, altitude: 0.01 }
  )
})

test("a link between opposite points is still one smooth arc", () => {
  assertArc(
    { lat: 0, lng: 60, altitude: 0.01 },
    { lat: 0, lng: -120, altitude: 0.01 }
  )
  assertArc(
    { lat: 31.2, lng: 121.5, altitude: 0.01 },
    { lat: -31.2, lng: -58.5, altitude: 0.01 }
  )
})

test("a second link between the same nodes arcs higher, ends unchanged", () => {
  const a: Anchor = { lat: 35.7, lng: 139.7, altitude: 0.01 }
  const b: Anchor = { lat: 37.5, lng: 127, altitude: 0.01 }
  const first = linkCurve(a, b, 32, 0)
  const second = linkCurve(a, b, 32, 1)
  assert.ok(close(first[0] ?? [], second[0] ?? []))
  assert.ok(close(first.at(-1) ?? [], second.at(-1) ?? []))
  assert.ok(
    Math.hypot(...(second[16] ?? [])) > Math.hypot(...(first[16] ?? []))
  )
})

test("history links are fainter than any current link", () => {
  const history = lineStyle("history").opacity
  for (const look of [
    "matched",
    "one_sided",
    "conflicting",
    "expired",
    "preparing",
    "failed",
    "closed",
  ] as const)
    assert.ok(lineStyle(look).opacity > history, look)
})

test("nodes too close to tell apart are one group, in the given order; the rest stay alone", () => {
  const at = (lat: number, lng: number): Anchor => ({
    lat,
    lng,
    altitude: 0.01,
  })
  const anchors = new Map<string, Anchor>([
    ["lax", at(43.6532, -79.3832)],
    ["nyc", at(43.6532, -79.3832)],
    ["sha", at(43.6532, -79.3832)],
    ["tokyo", at(35.68, 139.69)],
    ["yokohama", at(35.44, 139.64)],
    ["osaka", at(34.69, 135.5)],
  ])
  const order = ["lax", "nyc", "osaka", "sha", "tokyo", "unplaced", "yokohama"]
  const within = (radians: number) => (a: Anchor, b: Anchor) =>
    angleBetween(a, b) < radians
  // About 1.5° (Tokyo–Yokohama is 0.25°, Tokyo–Osaka 3.6°).
  assert.deepEqual(groupNearby(order, anchors, within(0.026)), [
    ["lax", "nyc", "sha"],
    ["osaka"],
    ["tokyo", "yokohama"],
  ])
  // Zoomed in, only the very same place stays together.
  assert.deepEqual(groupNearby(order, anchors, within(0.001)), [
    ["lax", "nyc", "sha"],
    ["osaka"],
    ["tokyo"],
    ["yokohama"],
  ])
  // A chain of close nodes is one group, even when its ends are far apart.
  const chain = new Map<string, Anchor>([
    ["a", at(0, 0)],
    ["c", at(0, 2)],
    ["b", at(0, 1)],
  ])
  assert.deepEqual(groupNearby(["a", "c", "b"], chain, within(0.02)), [
    ["a", "c", "b"],
  ])
  assert.equal(angleBetween(at(10, 20), at(10, 20)), 0)
})

test("a name is left out where it would cover another dot or a name already shown", () => {
  const at = (lat: number, lng: number): Anchor => ({
    lat,
    lng,
    altitude: 0.01,
  })
  const box = (key: string, anchor: Anchor) => ({
    key,
    anchor,
    dot: 4,
    left: 13,
    width: 60,
    height: 20,
  })
  // 1000 px per radian: 1° is about 17.5 px.
  const west = box("west", at(0, 0))
  const east = box("east", at(0, 3)) // 52 px right: under west's name
  const north = box("north", at(2, 0)) // 35 px up: clear of west
  const far = box("far", at(0, 10))
  assert.deepEqual([...fittingLabels([west, east, north, far], 1000)].sort(), [
    "east",
    "far",
    "north",
  ])
  // West's name covers east's dot, whatever the order; east's name covers nothing.
  assert.deepEqual([...fittingLabels([east, west, north, far], 1000)].sort(), [
    "east",
    "far",
    "north",
  ])
  // Across the antimeridian, the short way round.
  const [dx] = screenOffset(at(0, 179), at(0, -179), 1000)
  assert.ok(Math.abs(dx - 34.9) < 0.1)
})
