import assert from "node:assert/strict"
import { test } from "node:test"

import { attemptFor, outcomeUnknown } from "./attempt.ts"

let counter = 0
const fresh = () => `id-${++counter}`

test("a new attempt gets a fresh ID and its body built now", () => {
  const attempt = attemptFor(null, "Home", () => ({ name: "Home" }), fresh)
  assert.equal(attempt.key, "Home")
  assert.deepEqual(attempt.body, { name: "Home" })
  assert.match(attempt.requestId, /^id-/)
})

test("the same thing again after an unknown outcome is the same request", () => {
  const first = attemptFor(
    null,
    "Lab",
    () => ({ name: "Lab", expectedName: "Home" }),
    fresh
  )
  // Meanwhile the name read from the controller moved on; the body must not.
  const again = attemptFor(
    first,
    "Lab",
    () => ({ name: "Lab", expectedName: "Lab" }),
    fresh
  )
  assert.equal(again, first)
})

test("something else is a new attempt", () => {
  const first = attemptFor(null, "Lab", () => ({ name: "Lab" }), fresh)
  const other = attemptFor(first, "Office", () => ({ name: "Office" }), fresh)
  assert.notEqual(other.requestId, first.requestId)
  assert.deepEqual(other.body, { name: "Office" })
})

test("only a refusal settles an attempt", () => {
  const http = (status: number) => ({ kind: "http", status }) as const
  assert.equal(outcomeUnknown(http(400)), false)
  assert.equal(outcomeUnknown(http(409)), false)
  assert.equal(outcomeUnknown(http(503)), true)
  assert.equal(outcomeUnknown(http(504)), true)
  assert.equal(outcomeUnknown({ kind: "network" }), true)
  assert.equal(outcomeUnknown({ kind: "decode", status: 200 }), true)
})
