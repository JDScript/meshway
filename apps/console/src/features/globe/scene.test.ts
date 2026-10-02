import assert from "node:assert/strict"
import { test } from "node:test"

import { anchorsOf, cartesian, GLOBE_RADIUS, linkCurve, ORBIT, orbitPath, orbitPoint, pulseStreams, type SceneNode } from "./scene.ts"

const node = (id: string, name: string, placement: SceneNode["placement"] = null): SceneNode => ({
  id,
  name,
  placement,
  vitality: "quiet",
})

test("unplaced nodes sit evenly on the orbit in name order, never on the ground", () => {
  const anchors = anchorsOf([node("c", "sha"), node("a", "lax"), node("b", "nyc")])
  const lax = anchors.get("a")
  const nyc = anchors.get("b")
  assert.ok(lax && nyc)
  assert.equal(lax.orbit, true)
  assert.equal(lax.altitude, ORBIT.altitude)
  // First in name order starts at the ascending node.
  assert.equal(lax.lat, 0)
  assert.equal(lax.lng, ORBIT.ascendingLng)
  assert.notDeepEqual([nyc.lat, nyc.lng], [lax.lat, lax.lng])
})

test("only a sourced placement puts a node on the ground", () => {
  const placed = node("p", "tokyo", { lat: 35.7, lng: 139.7, source: "test", observedAt: "2026-10-02T00:00:00Z" })
  const anchors = anchorsOf([placed, node("u", "unknown")])
  assert.deepEqual(anchors.get("p"), { lat: 35.7, lng: 139.7, altitude: 0.01, orbit: false })
  assert.equal(anchors.get("u")?.orbit, true)
})

test("orbit longitudes stay continuous across the antimeridian", () => {
  for (let k = 1; k <= 64; k += 1) {
    const step = Math.abs(orbitPoint((k / 64) * 2 * Math.PI).lng - orbitPoint(((k - 1) / 64) * 2 * Math.PI).lng)
    assert.ok(step < 30, `jump of ${step}° at sample ${k}`)
  }
  const ring = orbitPath(64)
  const radius = GLOBE_RADIUS * (1 + ORBIT.altitude)
  for (const point of ring) assert.ok(Math.abs(Math.hypot(...point) - radius) < 1e-6)
})

test("a link curve joins its two anchors and stays above both the surface and its ends", () => {
  const a = { lat: 10, lng: -20, altitude: 0.01, orbit: false }
  const b = { lat: 40, lng: 100, altitude: ORBIT.altitude, orbit: true }
  const curve = linkCurve(a, b, 32)
  const first = curve[0]
  const last = curve.at(-1)
  assert.ok(first && last)
  const close = (p: number[], q: number[]) => p.every((value, i) => Math.abs(value - (q[i] ?? 0)) < 1e-6)
  assert.ok(close(first, cartesian(a.lat, a.lng, a.altitude)))
  assert.ok(close(last, cartesian(b.lat, b.lng, b.altitude)))
  for (const point of curve) assert.ok(Math.hypot(...point) >= GLOBE_RADIUS * (1 + a.altitude) - 1e-6)
  const middle = curve[16]
  assert.ok(middle && Math.hypot(...middle) > GLOBE_RADIUS * (1 + ORBIT.altitude))
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
