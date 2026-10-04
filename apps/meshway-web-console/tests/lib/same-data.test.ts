import assert from "node:assert/strict"
import { test } from "vitest"

import { sameData } from "@/lib/same-data"

test("plain values compare by content", () => {
  assert.ok(
    sameData({ a: 1, b: [1, { c: "x" }] }, { b: [1, { c: "x" }], a: 1 })
  )
  assert.ok(sameData(null, null))
  assert.ok(!sameData({ a: 1 }, { a: 2 }))
  assert.ok(!sameData([1, 2], [2, 1]))
  assert.ok(!sameData({ a: null }, {}))
  assert.ok(!sameData([1], { 0: 1 }))
})

test("an absent key and an undefined one are the same", () => {
  assert.ok(sameData({ a: 1, b: undefined }, { a: 1 }))
})
