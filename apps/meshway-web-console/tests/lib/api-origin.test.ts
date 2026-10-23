import assert from "node:assert/strict"
import { test } from "vitest"

import {
  apiOrigin,
  configuredApiOrigin,
  InvalidApiOrigin,
} from "@/lib/api-origin"

const OWN = "https://meshway.example.com"

test("without a configured API origin the console calls its own origin", () => {
  assert.equal(apiOrigin({}, OWN), OWN)
  assert.equal(apiOrigin({ VITE_MESHWAY_API_ORIGIN: "" }, OWN), OWN)
  assert.equal(apiOrigin({ VITE_MESHWAY_API_ORIGIN: "  " }, OWN), OWN)
})

test("a configured API origin is used as the bare origin", () => {
  for (const [value, origin] of [
    ["https://api.example.com", "https://api.example.com"],
    ["https://api.example.com/", "https://api.example.com"],
    [" https://API.example.com:8443 ", "https://api.example.com:8443"],
    ["https://api.example.com:443", "https://api.example.com"],
    ["http://127.0.0.1:8421", "http://127.0.0.1:8421"],
    ["http://localhost:8420/", "http://localhost:8420"],
    ["http://[::1]:8420", "http://[::1]:8420"],
  ] as const) {
    assert.equal(
      apiOrigin({ VITE_MESHWAY_API_ORIGIN: value }, OWN),
      origin,
      value
    )
  }
})

test("the development server keeps its proxied same origin", () => {
  assert.equal(
    apiOrigin(
      { DEV: true, VITE_MESHWAY_API_ORIGIN: "https://api.example.com" },
      "http://127.0.0.1:3001"
    ),
    "http://127.0.0.1:3001"
  )
})

test("anything but a secure bare origin is refused", () => {
  for (const value of [
    "api.example.com",
    "http://api.example.com",
    "ftp://api.example.com",
    "https://user@api.example.com",
    "https://user:pw@api.example.com",
    "https://api.example.com/api/v1",
    "https://api.example.com/?x=1",
    "https://api.example.com?",
    "https://api.example.com/#a",
    "https://api.example.com#",
  ])
    assert.throws(() => configuredApiOrigin(value), InvalidApiOrigin, value)
})
