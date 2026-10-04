import assert from "node:assert/strict"
import { test } from "vitest"

import { enrollmentKeyState, hostnameOf } from "@/lib/nodes/model"

const NOW = Date.parse("2026-10-03T12:00:00Z")
const key = (over: Partial<Parameters<typeof enrollmentKeyState>[0]> = {}) => ({
  consumedAt: null,
  consumedRequestId: null,
  revokedAt: null,
  expiresAt: "2026-10-03T13:00:00Z",
  ...over,
})

test("an unused key is pending until it expires or is revoked", () => {
  assert.deepEqual(enrollmentKeyState(key(), NOW), {
    state: "pending",
    at: "2026-10-03T13:00:00Z",
  })
  assert.deepEqual(
    enrollmentKeyState(key({ expiresAt: "2026-10-03T12:00:00Z" }), NOW),
    { state: "expired", at: "2026-10-03T12:00:00Z" }
  )
  assert.deepEqual(
    enrollmentKeyState(key({ revokedAt: "2026-10-03T11:00:00Z" }), NOW),
    { state: "revoked", at: "2026-10-03T11:00:00Z" }
  )
  // Revoked, then past its expiry: still revoked.
  assert.deepEqual(
    enrollmentKeyState(
      key({
        revokedAt: "2026-10-03T10:00:00Z",
        expiresAt: "2026-10-03T11:00:00Z",
      }),
      NOW
    ),
    { state: "revoked", at: "2026-10-03T10:00:00Z" }
  )
})

test("a used key stays used, at the time it was used", () => {
  assert.deepEqual(
    enrollmentKeyState(
      key({
        consumedAt: "2026-10-03T10:30:00Z",
        consumedRequestId: "8d6f0e8e-0000-4000-8000-000000000001",
        expiresAt: "2026-10-03T11:00:00Z",
      }),
      NOW
    ),
    { state: "consumed", at: "2026-10-03T10:30:00Z" }
  )
  // The request alone (its time not given) says it was used, too.
  assert.deepEqual(
    enrollmentKeyState(
      key({ consumedRequestId: "8d6f0e8e-0000-4000-8000-000000000001" }),
      NOW
    ),
    { state: "consumed", at: null }
  )
  // An omitted request and no time: not used.
  assert.equal(
    enrollmentKeyState(
      {
        consumedAt: null,
        revokedAt: null,
        expiresAt: "2026-10-03T13:00:00Z",
      },
      NOW
    ).state,
    "pending"
  )
})

test("a hostname is one DNS label, kept lowercase; empty is none", () => {
  assert.equal(hostnameOf("  Tokyo-1 "), "tokyo-1")
  assert.equal(hostnameOf(""), "")
  for (const bad of ["-a", "a-", "a.b", "a_b", "x".repeat(64), "東京"])
    assert.equal(hostnameOf(bad), null, bad)
})
