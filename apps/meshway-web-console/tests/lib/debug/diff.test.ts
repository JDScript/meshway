import assert from "node:assert/strict"
import { test } from "vitest"

import { differences } from "@/lib/debug/diff"

test("equal data has no differences", () => {
  assert.deepEqual(
    differences({ a: [1, { b: null }] }, { a: [1, { b: null }] }),
    {
      found: [],
      total: 0,
    }
  )
})

test("each differing value is named by its JSON Pointer", () => {
  assert.deepEqual(
    differences(
      { nodes: { "a/b": { phase: "up" } }, links: [1, 2] },
      { nodes: { "a/b": { phase: "down" } }, links: [1] }
    ),
    {
      found: [
        { path: "/links/1", cached: 2, read: undefined },
        { path: "/nodes/a~1b/phase", cached: "up", read: "down" },
      ],
      total: 2,
    }
  )
})

test("a member on one side only differs from nothing", () => {
  assert.deepEqual(differences({}, { asOf: "now" }).found, [
    { path: "/asOf", cached: undefined, read: "now" },
  ])
})

test("a value of another shape differs as a whole", () => {
  assert.deepEqual(differences({ a: [1] }, { a: { 0: 1 } }).found, [
    { path: "/a", cached: [1], read: { 0: 1 } },
  ])
})

test("past the limit, differences are counted, not listed", () => {
  const found = differences([1, 2, 3], [4, 5, 6], 2)
  assert.equal(found.found.length, 2)
  assert.equal(found.total, 3)
})
