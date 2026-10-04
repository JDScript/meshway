import assert from "node:assert/strict"
import { test } from "vitest"

import {
  bindingOf,
  entryCount,
  groupChanges,
  sectionOf,
  valueAt,
} from "@/lib/configuration/changes"

test("changes group by section in reading order", () => {
  const groups = groupChanges([
    { path: "/dns/magicDNS", kind: "changed" },
    { path: "/groups/group:a", kind: "added" },
    { path: "/access", kind: "changed" },
    { path: "/zeta", kind: "added" },
    { path: "/groups/group:b", kind: "removed" },
    { path: "/alpha", kind: "added" },
  ])
  assert.deepEqual(
    groups.map((group) => [group.section, group.changes.map((c) => c.path)]),
    [
      ["access", ["/access"]],
      ["groups", ["/groups/group:a", "/groups/group:b"]],
      ["dns", ["/dns/magicDNS"]],
      ["alpha", ["/alpha"]],
      ["zeta", ["/zeta"]],
    ]
  )
  assert.equal(sectionOf(""), "")
  assert.equal(sectionOf("/routing/egress"), "routing")
})

test("a change's values are read from each view by its pointer", () => {
  const view = {
    access: [{ src: ["*"] }],
    groups: { "group:a/b": ["x"] },
    dns: { magicDNS: false },
  }
  assert.deepEqual(valueAt(view, "/access/0/src"), {
    found: true,
    value: ["*"],
  })
  assert.deepEqual(valueAt(view, "/groups/group:a~1b"), {
    found: true,
    value: ["x"],
  })
  assert.deepEqual(valueAt(view, "/dns/magicDNS"), {
    found: true,
    value: false,
  })
  assert.deepEqual(valueAt(view, ""), { found: true, value: view })
  // Absent (added, removed, or a secret the view leaves out).
  assert.deepEqual(valueAt(view, "/access/1"), { found: false })
  assert.deepEqual(valueAt(view, "/access/01"), { found: false })
  assert.deepEqual(valueAt(view, "/nodes"), { found: false })
  assert.deepEqual(valueAt(view, "/dns/magicDNS/x"), { found: false })
  assert.deepEqual(valueAt(view, "/toString"), { found: false })
})

test("sections count their entries where they are lists or maps", () => {
  assert.equal(entryCount([1, 2]), 2)
  assert.equal(entryCount({ a: 1 }), 1)
  assert.equal(entryCount("x"), null)
  assert.equal(entryCount(null), null)
})

test("a change under /bindings names what the controller binds", () => {
  assert.equal(bindingOf("/bindings/ruleSets"), "ruleSets")
  assert.equal(bindingOf("/bindings/nodeUsers"), "nodeUsers")
  assert.equal(bindingOf("/bindings/somethingNew"), "other")
  assert.equal(bindingOf("/ruleSets/bindings"), null)
  assert.equal(bindingOf(""), null)
})
