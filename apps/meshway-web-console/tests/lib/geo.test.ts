import assert from "node:assert/strict"
import type { IpInfo } from "@meshway/control-client"
import { test } from "vitest"

import {
  asnText,
  countryName,
  geoText,
  hostOf,
  placeText,
  shortGeoText,
} from "@/lib/geo"

const DATASET = "a87ed054-27b2-4028-8f0a-726aa2d79f5e"
const AT = "2026-10-04T20:58:16.482Z"

const tokyo: IpInfo = {
  geoip: {
    status: "available",
    datasetId: DATASET,
    lookedUpAt: AT,
    data: {
      country: "JP",
      region: "Tokyo",
      city: "Shibuya City",
      position: {
        latitude: 35.6764,
        longitude: 139.65,
        accuracyRadiusKm: null,
      },
    },
  },
  asn: {
    status: "available",
    datasetId: DATASET,
    lookedUpAt: AT,
    data: {
      number: 63949,
      organization: "Akamai Technologies, Inc.",
      prefix: null,
    },
  },
}

test("a country is named in the reader's language; an unknown code stays as written", () => {
  assert.equal(countryName("JP"), "日本")
  assert.equal(countryName("not a code"), "not a code")
})

test("a place is most specific first, each name once", () => {
  assert.equal(
    placeText(tokyo.geoip.status === "available" ? tokyo.geoip.data : {}),
    "Shibuya City, Tokyo, 日本"
  )
  assert.equal(
    placeText({ city: "Singapore", region: "Singapore", country: "SG" }),
    "Singapore, 新加坡"
  )
  assert.equal(placeText({}), null)
})

test("an address's place and network, in full and short", () => {
  assert.equal(geoText(tokyo), "Shibuya City, Tokyo, 日本")
  assert.equal(asnText(tokyo), "AS63949 Akamai Technologies, Inc.")
  assert.equal(shortGeoText(tokyo), "日本 · AS63949")
  const privateAddress: IpInfo = {
    geoip: { status: "notApplicable", reason: "nonPublicAddress" },
    asn: { status: "notApplicable", reason: "nonPublicAddress" },
  }
  assert.equal(shortGeoText(privateAddress), null)
})

test("an endpoint's IP, with or without IPv6 brackets", () => {
  assert.equal(hostOf("139.162.70.1:8421"), "139.162.70.1")
  assert.equal(hostOf("[2001:db8::1]:443"), "2001:db8::1")
  assert.equal(hostOf("2001:db8::1"), "2001:db8::1")
})
