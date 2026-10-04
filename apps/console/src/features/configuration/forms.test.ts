import assert from "node:assert/strict"
import { test } from "node:test"

import { formViewOf } from "./forms.ts"

test("diagnostics go to the form that shows their field", () => {
  assert.equal(formViewOf("/access/2/ip"), "access")
  assert.equal(formViewOf("/defaults/topology"), "topology")
  assert.equal(formViewOf("/defaults/dialers/vless"), null)
  assert.equal(formViewOf("/groups/group:dev"), "groups")
  assert.equal(formViewOf("/nodes/abc/addresses/ipv4"), "addressPools")
  assert.equal(formViewOf("/nodes/abc/hostname"), null)
  assert.equal(formViewOf("/routing/mesh/0"), "routing")
  assert.equal(formViewOf("/ruleSets/office"), "ruleSets")
  assert.equal(formViewOf(""), null)
})
