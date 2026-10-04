import assert from "node:assert/strict"
import { test } from "vitest"

import {
  keyProblem,
  renamed,
  withEntry,
  without,
} from "@/lib/configuration/keyed"

test("renaming keeps the row's place and value", () => {
  const map = { "group:a": ["x"], "group:b": ["y"] }
  assert.deepEqual(Object.entries(renamed(map, "group:a", "group:c")), [
    ["group:c", ["x"]],
    ["group:b", ["y"]],
  ])
  assert.deepEqual(without(map, "group:a"), { "group:b": ["y"] })
  assert.deepEqual(Object.keys(withEntry(map, "group:0", [])), [
    "group:a",
    "group:b",
    "group:0",
  ])
})

test("a key needs a name of its own", () => {
  const map = { "group:a": [] }
  assert.equal(keyProblem(map, "group:", "", null), "empty")
  assert.equal(keyProblem(map, "group:", "a b", null), "space")
  assert.equal(keyProblem(map, "group:", "a", null), "taken")
  // Renaming a row to its own name is no clash.
  assert.equal(keyProblem(map, "group:", "a", "group:a"), null)
  assert.equal(keyProblem(map, "group:", "b", "group:a"), null)
  assert.equal(keyProblem(map, "", "__proto__", null), null)
})
