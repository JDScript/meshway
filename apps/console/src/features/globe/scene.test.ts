import assert from "node:assert/strict"
import { test } from "node:test"

import {
  anchorsOf,
  cartesian,
  GLOBE_RADIUS,
  linkCurve,
  pulseStreams,
  type Anchor,
  type SceneNode,
} from "./scene.ts"

const node = (
  id: string,
  name: string,
  placement: SceneNode["placement"] = null
): SceneNode => ({
  id,
  name,
  placement,
  vitality: "quiet",
})

const close = (p: readonly number[], q: readonly number[]) =>
  p.every((value, i) => Math.abs(value - (q[i] ?? 0)) < 1e-6)

test("only a sourced placement puts a node on the globe; unplaced nodes get no anchor", () => {
  const placed = node("p", "tokyo", {
    lat: 35.7,
    lng: 139.7,
    source: "test",
    observedAt: "2026-10-02T00:00:00Z",
  })
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

test("only confirmed links carry pulses, both ways, and never with reduced motion", () => {
  assert.equal(pulseStreams("one_sided", true).length, 0)
  assert.equal(pulseStreams("degraded", true).length, 0)
  assert.equal(pulseStreams("configured", true).length, 0)
  assert.equal(pulseStreams("confirmed", false).length, 0)
  assert.deepEqual(
    pulseStreams("confirmed", true).map((stream) => stream.reverse),
    [false, true]
  )
})
