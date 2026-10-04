import assert from "node:assert/strict"
import { test } from "node:test"

import {
  byEnrollmentKeyState,
  enrollmentKeyState,
  hostnameOf,
  identityWords,
} from "./model.ts"

const NOW = Date.parse("2026-10-03T12:00:00Z")
const key = (over: Partial<Parameters<typeof enrollmentKeyState>[0]> = {}) => ({
  consumedRequestId: null,
  revokedAt: null,
  expiresAt: "2026-10-03T13:00:00Z",
  ...over,
})

test("an enrolment key's state", () => {
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
  // Used stays used, even revoked afterwards or past its expiry.
  assert.deepEqual(
    enrollmentKeyState(
      key({
        consumedRequestId: "8d6f0e8e-0000-4000-8000-000000000001",
        revokedAt: "2026-10-03T11:30:00Z",
        expiresAt: "2026-10-03T11:00:00Z",
      }),
      NOW
    ),
    { state: "consumed", at: null }
  )
  // An omitted consumedRequestId is not used.
  assert.equal(
    enrollmentKeyState(
      { revokedAt: null, expiresAt: "2026-10-03T13:00:00Z" },
      NOW
    ).state,
    "pending"
  )
})

test("pending keys first, soonest to expire; then newest first", () => {
  const row = (
    state: "pending" | "revoked",
    createdAt: string,
    expiresAt: string
  ) => ({
    state,
    key: { createdAt, expiresAt },
  })
  const rows = [
    row("revoked", "2026-10-01T00:00:00Z", "2026-10-05T00:00:00Z"),
    row("pending", "2026-10-01T00:00:00Z", "2026-10-09T00:00:00Z"),
    row("revoked", "2026-10-02T00:00:00Z", "2026-10-05T00:00:00Z"),
    row("pending", "2026-10-02T00:00:00Z", "2026-10-04T00:00:00Z"),
  ].sort(byEnrollmentKeyState)
  assert.deepEqual(
    rows.map((r) => `${r.state} ${r.key.createdAt.slice(8, 10)}`),
    ["pending 02", "pending 01", "revoked 02", "revoked 01"]
  )
})

test("a hostname is one DNS label, kept lowercase; empty is none", () => {
  assert.equal(hostnameOf("  Tokyo-1 "), "tokyo-1")
  assert.equal(hostnameOf(""), "")
  for (const bad of ["-a", "a-", "a.b", "a_b", "x".repeat(64), "東京"])
    assert.equal(hostnameOf(bad), null, bad)
})

test("identity words", () => {
  assert.deepEqual(identityWords({ type: "tag", tags: ["tag:a", "tag:b"] }), [
    "tag:a",
    "tag:b",
  ])
  assert.deepEqual(identityWords({ type: "user", user: "a@example.com" }), [
    "a@example.com",
  ])
})
