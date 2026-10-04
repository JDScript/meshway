import assert from "node:assert/strict"
import { test } from "node:test"

import { landing, rememberedFor, rememberedValue } from "./remembered.ts"

test("a remembered network belongs to the account that opened it", () => {
  const stored = rememberedValue("user-a", "net-1")
  assert.equal(rememberedFor(stored, "user-a"), "net-1")
  assert.equal(rememberedFor(stored, "user-b"), null)
})

test("anything else stored is no preference", () => {
  assert.equal(rememberedFor(null, "user-a"), null)
  assert.equal(rememberedFor("net-1", "user-a"), null)
  assert.equal(rememberedFor('{"v":2,"user":"user-a"}', "user-a"), null)
})

test("`/` opens the remembered network when it is listed", () => {
  const listed = [{ id: "net-1" }, { id: "net-2" }]
  assert.deepEqual(landing(listed, false, "net-2"), {
    open: "net-2",
    check: null,
  })
  assert.deepEqual(landing(listed, false, null), { open: "net-1", check: null })
})

test("a remembered network no longer listed falls back to the first", () => {
  const listed = [{ id: "net-1" }]
  assert.deepEqual(landing(listed, false, "gone"), {
    open: "net-1",
    check: null,
  })
  // It may be on a later page: ask for it first.
  assert.deepEqual(landing(listed, true, "later"), {
    open: "net-1",
    check: "later",
  })
  assert.deepEqual(landing([], false, "gone"), { open: null, check: null })
})
