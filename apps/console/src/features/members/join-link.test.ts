import assert from "node:assert/strict"
import { test } from "node:test"

import { initialRole, joinLinkState, revocable } from "./join-link.ts"

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

test("revoking is offered only where the controller allows it on a pending link", () => {
  const allowed = { actions: { revoke: true } }
  const refused = { actions: { revoke: false } }
  assert.equal(revocable(allowed, "pending"), true)
  assert.equal(revocable(refused, "pending"), false)
  // Expired since the controller's read: nothing left to revoke.
  assert.equal(revocable(allowed, "expired"), false)
  assert.equal(revocable(allowed, "used"), false)
})

test("a new link starts as member when offered, else the lowest offered role", () => {
  assert.equal(initialRole(["owner", "admin", "member"]), "member")
  assert.equal(initialRole(["member"]), "member")
  assert.equal(initialRole(["owner", "admin"]), "admin")
  assert.equal(initialRole(["owner"]), "owner")
  assert.equal(initialRole([]), null)
})
