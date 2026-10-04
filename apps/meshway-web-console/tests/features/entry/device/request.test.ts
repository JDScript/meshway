import assert from "node:assert/strict"
import { test } from "vitest"

import {
  approverOf,
  awaitingDevice,
  keyGroups,
  offerFor,
  phaseAt,
  untilExpiry,
} from "@/features/entry/device/request"

const at = Date.parse("2026-10-03T12:00:00Z")
const later = "2026-10-03T12:10:00Z"
const earlier = "2026-10-03T11:59:59Z"

const me = "11111111-1111-4111-8111-111111111111"
const other = "22222222-2222-4222-8222-222222222222"
const netA = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
const netB = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"

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
  assert.equal(
    phaseAt({ phase: "cancelled", expiresAt: earlier }, at),
    "cancelled"
  )
})

test("only an approved request waits for the device", () => {
  assert.equal(awaitingDevice("approved"), true)
  for (const phase of [
    "pending",
    "withdrawn",
    "cancelled",
    "consumed",
    "expired",
  ] as const)
    assert.equal(awaitingDevice(phase), false)
})

test("the approver is this account, another, or not said", () => {
  assert.equal(approverOf(null, me), "unknown")
  assert.equal(approverOf({ userId: me, networkId: netA }, me), "self")
  assert.equal(approverOf({ userId: other, networkId: netA }, me), "other")
})

test("a pending request is approved into any of the account's networks, never denied", () => {
  const view = { request: { existing: null }, approval: null }
  assert.deepEqual(offerFor(view, "pending", me, [netA, netB]), {
    kind: "approve",
    networks: [netA, netB],
  })
  assert.deepEqual(offerFor(view, "pending", me, []), {
    kind: "approve",
    networks: [],
  })
})

test("a re-login is approved only into the node's own network", () => {
  const view = { request: { existing: { networkId: netB } }, approval: null }
  assert.deepEqual(offerFor(view, "pending", me, [netA, netB]), {
    kind: "approve",
    networks: [netB],
  })
  assert.deepEqual(offerFor(view, "pending", me, [netA]), {
    kind: "approve",
    networks: [],
  })
})

test("only the approver may withdraw, and only while approved", () => {
  const mine = {
    request: { existing: null },
    approval: { userId: me, networkId: netA },
  }
  const theirs = {
    request: { existing: null },
    approval: { userId: other, networkId: netA },
  }
  const hidden = { request: { existing: null }, approval: null }
  assert.deepEqual(offerFor(mine, "approved", me, [netA]), {
    kind: "withdraw",
  })
  assert.deepEqual(offerFor(theirs, "approved", me, [netA]), { kind: "none" })
  assert.deepEqual(offerFor(hidden, "approved", me, [netA]), { kind: "none" })
  for (const phase of [
    "withdrawn",
    "cancelled",
    "consumed",
    "expired",
  ] as const)
    assert.deepEqual(offerFor(mine, phase, me, [netA]), { kind: "none" })
})

test("the time to expiry never goes below zero", () => {
  assert.equal(untilExpiry(later, at), 600_000)
  assert.equal(untilExpiry(earlier, at), 0)
})

test("a key is grouped in fours", () => {
  assert.equal(keyGroups("ABCDef0123"), "abcd ef01 23")
  assert.equal(keyGroups(""), "")
})
