import assert from "node:assert/strict"
import { test } from "vitest"

import { actorId, actorText, type ActorWords } from "@/lib/activity"

const me = "11111111-2222-4333-8444-555555555555"
const other = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
const words: ActorWords = {
  system: () => "system",
  node: (id) => `node ${id}`,
  user: (id) => `user ${id}`,
  self: (name) => `${name} (you)`,
}

test("a user actor is named by the email recorded with the event", () => {
  assert.equal(
    actorText(
      { type: "user", userId: other, email: "old@example.com" },
      me,
      words
    ),
    "old@example.com"
  )
  assert.equal(
    actorText(
      { type: "user", userId: me, email: "old@example.com" },
      me,
      words
    ),
    "old@example.com (you)"
  )
})

test("without a snapshot the user ID says who, never a current address", () => {
  assert.equal(
    actorText({ type: "user", userId: other, email: null }, me, words),
    "user aaaaaaaa"
  )
  assert.equal(
    actorText({ type: "user", userId: me, email: null }, me, words),
    "user 11111111 (you)"
  )
  assert.equal(
    actorText({ type: "user", userId: me, email: null }, null, words),
    "user 11111111"
  )
})

test("the system is the system", () => {
  assert.equal(actorText({ type: "system" }, me, words), "system")
  assert.equal(actorId({ type: "system" }), undefined)
  assert.equal(actorId({ type: "user", userId: other, email: null }), other)
})

test("a Node acting for itself is named by the start of its ID", () => {
  assert.equal(
    actorText({ type: "node", nodeId: other }, me, words),
    "node aaaaaaaa"
  )
  assert.equal(actorId({ type: "node", nodeId: other }), undefined)
})
