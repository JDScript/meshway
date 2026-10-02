import assert from "node:assert/strict"
import { test } from "node:test"

import type { MeshAccessSnapshot } from "@meshway/control-client"
import { zMeshAccessPolicy, zMeshAccessSnapshot } from "@meshway/control-client/schemas"

import { accessBase, type StoredAccessDraft } from "./access-draft-registry.ts"
import { DraftVault } from "./draft-vault.ts"
import { newManagedForm } from "./managed-listener-model.ts"
import { editText, openDocument, storeDocument } from "./policy-document.ts"

const FIRST = "a".repeat(32)
const SECOND = "b".repeat(32)

function snapshot(revision: number): MeshAccessSnapshot {
  return zMeshAccessSnapshot.parse({
    revision,
    policy: { default_action: "allow", rules: [] },
    source_jsonc: null,
    valid: true,
    diagnostics: [],
  })
}

/** An invalid, commented JSONC draft on top of a given CAS base. */
function invalidDraft(base: MeshAccessSnapshot, text: string): StoredAccessDraft {
  const document = editText(openDocument(accessBase(base), zMeshAccessPolicy), text, zMeshAccessPolicy)
  return { base, inventory: [], tags: { revision: base.revision, tags: [], assignments: [] }, document: storeDocument(document), builder: null, nextSession: 0, view: "jsonc" }
}

test("each network keeps its own draft and CAS base; revisits return the same registries", () => {
  const vault = new DraftVault()
  const first = vault.drafts(FIRST)
  assert.equal(vault.drafts(FIRST), first)
  const second = vault.drafts(SECOND)
  assert.notEqual(second.access, first.access)

  // Both networks can be at the same revision: revision alone never identifies a draft.
  const owner = second.access.claim()
  second.access.write(owner, invalidDraft(snapshot(3), "// second network\n{ invalid"))
  assert.equal(vault.hasDrafts(SECOND), true)
  assert.equal(vault.hasDrafts(FIRST), false)
  assert.equal(first.access.read(), null)

  // Leaving and returning (a new editor mount) restores text, comments and base.
  second.access.release(owner)
  const restored = vault.drafts(SECOND).access.read()
  assert.ok(restored)
  assert.equal(restored.base.revision, 3)
  assert.equal(restored.document.text, "// second network\n{ invalid")
  assert.equal(restored.view, "jsonc")
})

test("clearing one network leaves the others; logout clears every network", () => {
  const vault = new DraftVault()
  const first = vault.drafts(FIRST)
  const second = vault.drafts(SECOND)
  const firstOwner = first.access.claim()
  const secondOwner = second.access.claim()
  first.access.write(firstOwner, invalidDraft(snapshot(1), "// first"))
  second.access.write(secondOwner, invalidDraft(snapshot(1), "// second"))
  first.access.clear(firstOwner)
  assert.equal(vault.hasDrafts(FIRST), false)
  second.managedListeners.put({ key: "enroll", kind: "enroll", nodeId: null, baseRevision: null, original: null, initial: newManagedForm("vless_tcp", []), form: newManagedForm("vless_websocket", []), idFollows: true, open: true })
  assert.equal(vault.hasDrafts(FIRST), false, "a managed listener form belongs to its own network")
  assert.equal(vault.hasDrafts(SECOND), true)
  assert.equal(vault.anyDrafts(), true)
  vault.reset()
  assert.equal(vault.anyDrafts(), false)
  assert.equal(second.access.read(), null)
  assert.equal(second.managedListeners.hasDraft(), false)
  assert.equal(vault.hasDrafts("c".repeat(32)), false)
})
