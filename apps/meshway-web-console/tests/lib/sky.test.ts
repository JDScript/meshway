import assert from "node:assert/strict"
import { test } from "vitest"

import {
  isDay,
  nextTurn,
  sublunarPoint,
  subsolarPoint,
  sunAt,
  type GeoPoint,
} from "@/lib/sky/astronomy"
import { placeOfZone } from "@/lib/sky/place"
import { skyOf } from "@/lib/sky/sky"

const at = (iso: string) => Date.parse(iso)
const near = (actual: number, expected: number, within: number) =>
  assert.ok(
    Math.abs(actual - expected) <= within,
    `${actual} is not within ${within} of ${expected}`
  )

/** The angle (degrees) between two points, seen from the Earth's centre. */
function apart(a: GeoPoint, b: GeoPoint) {
  const r = Math.PI / 180
  return (
    Math.acos(
      Math.sin(a.lat * r) * Math.sin(b.lat * r) +
        Math.cos(a.lat * r) *
          Math.cos(b.lat * r) *
          Math.cos((a.lng - b.lng) * r)
    ) / r
  )
}

const TORONTO = { lat: 43.7, lng: -79.4 }

test("the sun is overhead of the tropic at the June solstice, of the equator at the March equinox", () => {
  const june = subsolarPoint(at("2026-06-21T12:00:00Z"))
  near(june.lat, 23.44, 0.05)
  near(june.lng, 0, 1)
  near(subsolarPoint(at("2026-03-20T14:46:00Z")).lat, 0, 0.05)
})

test("the moon is with the sun at an eclipsed new moon and opposite it at an eclipsed full moon", () => {
  const newMoon = at("2026-02-17T12:01:00Z")
  const fullMoon = at("2026-03-03T11:38:00Z")
  assert.ok(apart(subsolarPoint(newMoon), sublunarPoint(newMoon)) < 3)
  assert.ok(apart(subsolarPoint(fullMoon), sublunarPoint(fullMoon)) > 177)
})

test("Toronto's sun: high at its June noon, set at midnight, rising at 5:36", () => {
  near(sunAt(at("2026-06-21T17:22:00Z"), TORONTO).altitude, 69.7, 0.3)
  assert.equal(isDay(at("2026-06-21T04:00:00Z"), TORONTO), false)
  assert.equal(sunAt(at("2026-06-21T13:00:00Z"), TORONTO).rising, true)
  near(
    nextTurn(at("2026-06-21T04:00:00Z"), TORONTO) ?? 0,
    at("2026-06-21T09:36:00Z"),
    3 * 60_000
  )
  // Svalbard's midnight sun: no sunset within two days.
  assert.equal(
    nextTurn(at("2026-06-21T00:00:00Z"), { lat: 78.2, lng: 15.6 }),
    null
  )
})

test("a time zone is placed at its city, old names included; one placed nowhere is not", () => {
  assert.deepEqual(placeOfZone("America/Toronto"), { lat: 43.7, lng: -79.4 })
  assert.deepEqual(placeOfZone("Asia/Calcutta"), placeOfZone("Asia/Kolkata"))
  assert.equal(placeOfZone("UTC"), null)
  assert.equal(placeOfZone("Etc/GMT+3"), null)
})

test("the sky: full day, full night, and a glow while the sun is about the horizon", () => {
  assert.deepEqual(skyOf({ altitude: 30, rising: false }), {
    daylight: 1,
    glow: 0,
    dawn: false,
  })
  assert.deepEqual(skyOf({ altitude: -20, rising: true }), {
    daylight: 0,
    glow: 0,
    dawn: true,
  })
  const dusk = skyOf({ altitude: -2, rising: false })
  assert.ok(dusk.glow > 0.9 && dusk.daylight > 0 && dusk.daylight < 0.5)
})
