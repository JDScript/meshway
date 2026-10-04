import assert from "node:assert/strict"
import { test } from "node:test"

import {
  awaitingDevice,
  decidable,
  keyGroups,
  phaseAt,
  untilExpiry,
} from "./request.ts"

const at = Date.parse("2026-10-03T12:00:00Z")
const later = "2026-10-03T12:10:00Z"
const earlier = "2026-10-03T11:59:59Z"

test("an open request past its expiry shows as expired", () => {
  assert.equal(phaseAt({ phase: "pending", expiresAt: later }, at), "pending")
  assert.equal(phaseAt({ phase: "pending", expiresAt: earlier }, at), "expired")
  assert.equal(
    phaseAt({ phase: "approved", expiresAt: earlier }, at),
    "expired"
  )
})

test("a settled request keeps the controller's phase after its expiry", () => {
  assert.equal(
    phaseAt({ phase: "consumed", expiresAt: earlier }, at),
    "consumed"
  )
  assert.equal(
    phaseAt({ phase: "withdrawn", expiresAt: earlier }, at),
    "withdrawn"
  )
})

test("only a pending request is decided here; an approved one waits for the device", () => {
  assert.equal(decidable("pending"), true)
  for (const phase of [
    "approved",
    "withdrawn",
    "cancelled",
    "consumed",
    "expired",
  ] as const)
    assert.equal(decidable(phase), false)
  assert.equal(awaitingDevice("approved"), true)
  assert.equal(awaitingDevice("pending"), false)
})

test("the time to expiry never goes below zero", () => {
  assert.equal(untilExpiry(later, at), 600_000)
  assert.equal(untilExpiry(earlier, at), 0)
})

test("a key is grouped in fours", () => {
  assert.equal(keyGroups("ABCDef0123"), "abcd ef01 23")
  assert.equal(keyGroups(""), "")
})
