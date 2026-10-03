import assert from "node:assert/strict"
import { test } from "node:test"

import { bestMatches, matchScore } from "./match.ts"

const ID = "a".repeat(64)

test("an empty search matches everything", () => {
  assert.equal(matchScore("  ", ["lax"]), 1)
})

test("every word typed must be found", () => {
  assert.ok(matchScore("lax edge", ["lax-1", "10.42.0.2", "edge"]) > 0)
  assert.equal(matchScore("lax hub", ["lax-1", "10.42.0.2", "edge"]), 0)
})

test("substrings only: no fuzzy match inside an ID", () => {
  assert.equal(matchScore("ab", ["lax", ID]), 0)
  assert.ok(matchScore("aaaa", ["lax", ID]) > 0)
})

test("a match on the name, or at a keyword's start, ranks first", () => {
  const entries = [
    { name: "b", keywords: ["sha", "10.0.0.3", "tag-lax"] },
    { name: "a", keywords: ["xlax", "10.0.0.2"] },
    { name: "c", keywords: ["lax", "10.0.0.1"] },
  ]
  assert.deepEqual(
    bestMatches(entries, "lax", 10).shown.map((entry) => entry.name),
    ["c", "a", "b"]
  )
})

test("results are bounded, and the rest counted", () => {
  const entries = Array.from({ length: 9 }, (_, index) => ({
    keywords: [`node-${index}`],
  }))
  const { shown, more } = bestMatches(entries, "node", 6)
  assert.equal(shown.length, 6)
  assert.equal(more, 3)
  assert.deepEqual(bestMatches(entries, "zzz", 6), { shown: [], more: 0 })
})
