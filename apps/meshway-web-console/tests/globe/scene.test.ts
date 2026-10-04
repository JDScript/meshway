import assert from "node:assert/strict"
import { test } from "vitest"

import {
  anchorsOf,
  cartesian,
  GLOBE_RADIUS,
  lineStyle,
  linkCurve,
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
