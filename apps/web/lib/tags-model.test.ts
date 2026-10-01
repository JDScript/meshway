import assert from "node:assert/strict"
import { test } from "node:test"

import type { MeshAccessPolicy, TagSnapshot } from "@meshway/control-client"

import { catalogKey, deletionBlocker, normalizeTagInput, validateCatalog } from "./tags-model.ts"

const snapshot: TagSnapshot = {
  revision: 3,
  tags: [
    { tag: "tag:edge", description: "" },
    { tag: "tag:core", description: "" },
    { tag: "tag:spare", description: "" },
  ],
  assignments: [{ node_id: "n", tags: ["tag:edge"] }],
}

test("catalog names follow the selector pattern and are unique", () => {
  assert.deepEqual(validateCatalog([{ tag: "tag:ok_1-x", description: "" }]), [])
  assert.deepEqual(
    validateCatalog([
      { tag: "tag:Bad", description: "" },
      { tag: "tag:a", description: "" },
      { tag: "tag:a", description: "x".repeat(257) },
    ]).map((issue) => [issue.index, issue.field]),
    [
      [0, "tag"],
      [2, "tag"],
      [2, "description"],
    ]
  )
  assert.equal(normalizeTagInput("  Prod "), "tag:prod")
  assert.equal(normalizeTagInput("tag:prod"), "tag:prod")
})

test("deleting an assigned or referenced tag is explained before the controller refuses", () => {
  const access: MeshAccessPolicy = {
    default_action: "reject",
    rules: [{ id: "r", source: ["tag:core"], destination: ["*"], action: "allow", bidirectional: false }],
  }
  assert.match(deletionBlocker("tag:edge", snapshot, access) ?? "", /1 个节点/)
  assert.match(deletionBlocker("tag:core", snapshot, access) ?? "", /1 条/)
  assert.equal(deletionBlocker("tag:spare", snapshot, access), null)
})

test("catalog identity ignores order, as the controller sorts it", () => {
  assert.equal(catalogKey(snapshot.tags), catalogKey([...snapshot.tags].reverse()))
})
