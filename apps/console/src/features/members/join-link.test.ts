import assert from "node:assert/strict"
import { test } from "node:test"

import { byJoinLinkState, joinLinkState } from "./join-link.ts"

const now = Date.parse("2026-10-03T12:00:00Z")
const link = {
  consumedAt: null,
  revokedAt: null,
  expiresAt: "2026-10-04T12:00:00Z",
}

test("a link's state follows its times", () => {
  assert.deepEqual(joinLinkState(link, now), {
    state: "pending",
    at: link.expiresAt,
  })
  assert.deepEqual(
    joinLinkState({ ...link, expiresAt: "2026-10-03T12:00:00Z" }, now),
    { state: "expired", at: "2026-10-03T12:00:00Z" }
  )
  assert.deepEqual(
    joinLinkState({ ...link, revokedAt: "2026-10-03T11:00:00Z" }, now),
    { state: "revoked", at: "2026-10-03T11:00:00Z" }
  )
  // Used is final, whatever came after it.
  assert.deepEqual(
    joinLinkState(
      {
        consumedAt: "2026-10-03T10:00:00Z",
        revokedAt: null,
        expiresAt: "2026-10-03T11:00:00Z",
      },
      now
    ),
    { state: "used", at: "2026-10-03T10:00:00Z" }
  )
  // Revoked after expiring stays revoked.
  assert.equal(
    joinLinkState(
      {
        consumedAt: null,
        revokedAt: "2026-10-03T11:30:00Z",
        expiresAt: "2026-10-03T11:00:00Z",
      },
      now
    ).state,
    "revoked"
  )
})

test("pending links come first, soonest first; the rest latest first", () => {
  const rows = [
    { state: "used", at: "2026-10-01T00:00:00Z" },
    { state: "pending", at: "2026-10-05T00:00:00Z" },
    { state: "revoked", at: "2026-10-02T00:00:00Z" },
    { state: "pending", at: "2026-10-04T00:00:00Z" },
  ] as const
  assert.deepEqual(
    [...rows].sort(byJoinLinkState).map((row) => `${row.state} ${row.at}`),
    [
      "pending 2026-10-04T00:00:00Z",
      "pending 2026-10-05T00:00:00Z",
      "revoked 2026-10-02T00:00:00Z",
      "used 2026-10-01T00:00:00Z",
    ]
  )
})
