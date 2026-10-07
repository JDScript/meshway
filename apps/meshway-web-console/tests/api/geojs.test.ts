import assert from "node:assert/strict"
import { afterEach, test, vi } from "vitest"

import { lookUpIp } from "@/api/geojs"

afterEach(() => {
  vi.unstubAllGlobals()
})

/** geojs's answers as it writes them (an excerpt of a real response). */
const ANSWERS = [
  {
    accuracy: 1000,
    asn: 15169,
    country: "United States",
    country_code: "US",
    ip: "8.8.8.8",
    latitude: "37.751",
    longitude: "-97.822",
    organization: "AS15169 Google LLC",
    organization_name: "Google LLC",
    timezone: "America/Chicago",
  },
  {
    asn: 13335,
    ip: "2606:4700:4700::1111",
    latitude: "nil",
    longitude: "nil",
    organization: "AS13335 Cloudflare, Inc.",
    organization_name: "Cloudflare, Inc.",
  },
  {
    asn: 64512,
    ip: "198.51.100.1",
    latitude: "nil",
    longitude: "nil",
    organization: "AS64512 Unknown",
    organization_name: "Unknown",
  },
]

test("addresses asked for together go in one request, without credentials or a referrer", async () => {
  const asked: { url: URL; init: RequestInit | undefined }[] = []
  vi.stubGlobal("fetch", (url: URL, init?: RequestInit) => {
    asked.push({ url, init })
    return Promise.resolve(Response.json(ANSWERS))
  })
  const [google, cloudflare, unknown] = await Promise.all([
    lookUpIp("8.8.8.8"),
    lookUpIp("2606:4700:4700::1111"),
    lookUpIp("198.51.100.1"),
    lookUpIp("8.8.8.8"),
  ])
  assert.equal(asked.length, 1)
  assert.equal(
    asked[0]?.url.searchParams.get("ip"),
    "8.8.8.8,2606:4700:4700::1111,198.51.100.1"
  )
  assert.equal(asked[0]?.init?.credentials, "omit")
  assert.equal(asked[0]?.init?.referrerPolicy, "no-referrer")
  assert.deepEqual(google, {
    country: "US",
    region: null,
    city: null,
    position: { lat: 37.751, lng: -97.822, accuracyKm: 1000 },
    asn: { number: 15169, organization: "Google LLC" },
  })
  // "nil" coordinates are none; the network is still known.
  assert.equal(cloudflare?.position, null)
  assert.deepEqual(cloudflare?.asn, {
    number: 13335,
    organization: "Cloudflare, Inc.",
  })
  // AS64512 "Unknown" is geojs's word for not knowing.
  assert.deepEqual(unknown, {
    country: null,
    region: null,
    city: null,
    position: null,
    asn: null,
  })
})

test("an error, an answer of another shape or a missing address fails the lookup", async () => {
  vi.stubGlobal("fetch", () =>
    Promise.resolve(new Response("busy", { status: 503 }))
  )
  await assert.rejects(lookUpIp("8.8.8.8"))
  vi.stubGlobal("fetch", () =>
    Promise.resolve(Response.json([{ ip: "8.8.8.8", latitude: 37.7 }]))
  )
  await assert.rejects(lookUpIp("8.8.8.8"))
  vi.stubGlobal("fetch", () => Promise.resolve(Response.json([])))
  await assert.rejects(lookUpIp("8.8.8.8"))
})
