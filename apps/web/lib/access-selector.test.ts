import assert from "node:assert/strict"
import { test } from "node:test"

import { parseSelector, reversed, selectorListIssues } from "./access-selector.ts"

const ID = "a".repeat(64)

test("parses the three selector forms", () => {
  assert.deepEqual(parseSelector("*"), { kind: "all" })
  assert.deepEqual(parseSelector(`node:${ID}`), { kind: "node", id: ID })
  assert.deepEqual(parseSelector("tag:prod"), { kind: "tag", tag: "tag:prod", name: "prod" })
  assert.deepEqual(parseSelector("tag:a_b-1"), { kind: "tag", tag: "tag:a_b-1", name: "a_b-1" })
  for (const raw of ["", "node:", "node:ABC", `node:${"A".repeat(64)}`, "tag:Prod", "tag:", "prod", "**", "tag:-x", `tag:${"a".repeat(49)}`]) {
    assert.equal(parseSelector(raw).kind, "invalid", raw)
  }
})

test("* must stand alone and lists are non-empty and unique", () => {
  assert.deepEqual(selectorListIssues(["*"]), [])
  assert.deepEqual(selectorListIssues(["tag:prod", `node:${ID}`]), [])
  assert.equal(selectorListIssues([]).length, 1)
  assert.equal(selectorListIssues(Array.from({ length: 33 }, (_, index) => `tag:t${index}`)).length, 1)
  assert.deepEqual(
    selectorListIssues(["tag:prod", "*"]).map((issue) => issue.index),
    [1]
  )
  assert.deepEqual(
    selectorListIssues(["tag:a", "tag:a"]).map((issue) => issue.index),
    [1]
  )
})

test("reverse swaps the lists without copying the rule", () => {
  assert.deepEqual(reversed({ source: ["tag:a"], destination: ["*"] }), { source: ["*"], destination: ["tag:a"] })
})
