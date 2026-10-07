import assert from "node:assert/strict"
import { afterEach, beforeEach, test, vi } from "vitest"

import type { IpGeo } from "@/api/geojs"

const TOKYO: IpGeo = {
  country: "JP",
  region: "Tokyo",
  city: null,
  position: { lat: 35.7, lng: 139.7, accuracyKm: 50 },
  asn: { number: 2516, organization: "KDDI CORPORATION" },
}

/** A browser's localStorage, kept in a map. */
function storage(): Storage {
  const items = new Map<string, string>()
  return {
    get length() {
      return items.size
    },
    clear: () => items.clear(),
    getItem: (key) => items.get(key) ?? null,
    key: (index) => [...items.keys()][index] ?? null,
    removeItem: (key) => void items.delete(key),
    setItem: (key, value) => void items.set(key, value),
  }
}

beforeEach(() => {
  vi.resetModules()
  vi.stubGlobal("window", { localStorage: storage() })
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

test("an answer is kept for a week, then asked again", async () => {
  vi.useFakeTimers({ now: Date.parse("2026-10-07T00:00:00Z") })
  const { keepGeo, keptGeo } = await import("@/state/geo-lookup")
  assert.equal(keptGeo("203.0.113.7"), undefined)
  keepGeo("203.0.113.7", TOKYO)
  assert.deepEqual(keptGeo("203.0.113.7")?.geo, TOKYO)
  vi.setSystemTime(Date.parse("2026-10-13T23:59:00Z"))
  assert.deepEqual(keptGeo("203.0.113.7")?.geo, TOKYO)
  vi.setSystemTime(Date.parse("2026-10-14T00:00:00Z"))
  assert.equal(keptGeo("203.0.113.7"), undefined)
})

test("kept answers are forgotten; turning the lookups off forgets them too", async () => {
  const { forgetGeo, keepGeo, keptGeo, setGeoLookup } =
    await import("@/state/geo-lookup")
  keepGeo("203.0.113.7", TOKYO)
  forgetGeo()
  assert.equal(keptGeo("203.0.113.7"), undefined)
  keepGeo("203.0.113.7", TOKYO)
  setGeoLookup(false)
  assert.equal(keptGeo("203.0.113.7"), undefined)
  assert.equal(window.localStorage.getItem("meshway.geo-lookup"), "off")
  setGeoLookup(true)
  assert.equal(window.localStorage.getItem("meshway.geo-lookup"), null)
})

test("answers stored in another shape are not used", async () => {
  window.localStorage.setItem(
    "meshway.geo",
    JSON.stringify({ v: 1, answers: { "203.0.113.7": { at: 1, geo: {} } } })
  )
  const { keptGeo } = await import("@/state/geo-lookup")
  assert.equal(keptGeo("203.0.113.7"), undefined)
})
