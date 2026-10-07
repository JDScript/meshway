import assert from "node:assert/strict"
import type { DerivedNodeState } from "@meshway/control-client"
import { test } from "vitest"

import type { IpGeo } from "@/api/geojs"
import {
  asnText,
  countryName,
  geoText,
  hostOf,
  nodeAddresses,
  placeText,
  shortGeoText,
  type AddressGeo,
} from "@/lib/geo"

const NOTHING: IpGeo = {
  country: null,
  region: null,
  city: null,
  position: null,
  asn: null,
}

const tokyo: AddressGeo = {
  kind: "found",
  geo: {
    country: "JP",
    region: "Tokyo",
    city: "Shibuya City",
    position: { lat: 35.6764, lng: 139.65, accuracyKm: 20 },
    asn: { number: 63949, organization: "Akamai Technologies, Inc." },
  },
}

test("a country is named in the reader's language; an unknown code stays as written", () => {
  assert.equal(countryName("JP"), "日本")
  assert.equal(countryName("not a code"), "not a code")
})

test("a place is most specific first, each name once", () => {
  assert.equal(
    placeText(tokyo.kind === "found" ? tokyo.geo : NOTHING),
    "Shibuya City, Tokyo, 日本"
  )
  assert.equal(
    placeText({
      ...NOTHING,
      city: "Singapore",
      region: "Singapore",
      country: "SG",
    }),
    "Singapore, 新加坡"
  )
  assert.equal(placeText(NOTHING), null)
})

test("an address's place and network, in full and short, or why they are not known", () => {
  assert.equal(geoText(tokyo), "Shibuya City, Tokyo, 日本")
  assert.equal(asnText(tokyo), "AS63949 Akamai Technologies, Inc.")
  assert.equal(shortGeoText(tokyo), "日本 · AS63949")
  const unknown: AddressGeo = { kind: "found", geo: NOTHING }
  assert.equal(geoText(unknown), "没有地区信息")
  assert.equal(asnText(unknown), "查不到 ASN")
  assert.equal(shortGeoText(unknown), null)
  assert.equal(geoText({ kind: "notPublic" }), "非公网地址")
  assert.equal(asnText({ kind: "notPublic" }), "ASN 不适用")
  assert.equal(asnText({ kind: "off" }), "本浏览器不查询位置")
  assert.equal(shortGeoText({ kind: "pending" }), null)
})

test("a node's addresses: those the server does not look up are not public, the location's is", () => {
  const derived: DerivedNodeState = {
    ipInfo: {
      "192.168.1.20": {
        geoip: { status: "notApplicable", reason: "nonPublicAddress" },
        asn: { status: "notApplicable", reason: "nonPublicAddress" },
      },
      "198.51.100.42": {
        geoip: { status: "unavailable", reason: "datasetUnavailable" },
        asn: { status: "unavailable", reason: "datasetUnavailable" },
      },
    },
    locationAbsent: "datasetUnavailable",
  }
  const addresses = nodeAddresses(derived)
  assert.equal(addresses.location, null)
  assert.equal(addresses.locationAbsent, "datasetUnavailable")
  assert.deepEqual(
    [...addresses.scopes],
    [
      ["192.168.1.20", "notPublic"],
      ["198.51.100.42", "public"],
    ]
  )
  const located = nodeAddresses({
    ipInfo: {},
    location: {
      address: "198.51.100.42",
      data: {},
      datasetId: "a87ed054-27b2-4028-8f0a-726aa2d79f5e",
      endpointSource: "stun",
      observedAt: "2026-10-03T12:00:00Z",
      expiresAt: "2026-10-03T12:00:30Z",
      sourceGeneration: 1,
      sourceSocketId: "s1",
    },
  })
  assert.deepEqual(located.location, {
    address: "198.51.100.42",
    endpointSource: "stun",
    observedAt: "2026-10-03T12:00:00Z",
    expiresAt: "2026-10-03T12:00:30Z",
  })
  assert.equal(located.scopes.get("198.51.100.42"), "public")
})

test("an endpoint's IP, with or without IPv6 brackets", () => {
  assert.equal(hostOf("139.162.70.1:8421"), "139.162.70.1")
  assert.equal(hostOf("[2001:db8::1]:443"), "2001:db8::1")
  assert.equal(hostOf("2001:db8::1"), "2001:db8::1")
})
