import assert from "node:assert/strict"
import { test } from "vitest"

import { joinRefusal } from "@/lib/join-refusal"

test("a link's refusal is named only by the link's own codes", () => {
  assert.equal(joinRefusal("invalid_join_link"), "invalid")
  assert.equal(joinRefusal("join_link_used"), "used")
  assert.equal(joinRefusal("join_link_revoked"), "revoked")
  assert.equal(joinRefusal("join_link_expired"), "expired")
  assert.equal(
    joinRefusal("join_link_issuer_unavailable"),
    "issuer_unavailable"
  )
  // Not about the link: the card keeps it and says what went wrong.
  assert.equal(joinRefusal("already_member"), null)
  assert.equal(joinRefusal("busy"), null)
  assert.equal(joinRefusal("constructor"), null)
  assert.equal(joinRefusal(null), null)
})
