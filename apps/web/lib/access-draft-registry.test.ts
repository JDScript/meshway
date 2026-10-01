import assert from "node:assert/strict"
import { test } from "node:test"

import type { MeshAccessPolicy, MeshAccessSnapshot } from "@meshway/control-client"

import { AccessDraftRegistry, type StoredAccessDraft } from "./access-draft-registry.ts"
import { accessKey, newGrantForm, type WorkingGrant } from "./access-model.ts"

const A = "a".repeat(64)
const B = "b".repeat(64)
const G1 = "1".repeat(64)

const policy: MeshAccessPolicy = { grants: [{ id: G1, path: [A, B] }] }

function snapshot(revision: number, value: MeshAccessPolicy = policy): MeshAccessSnapshot {
  return { revision, policy: value, valid: true, diagnostics: [] }
}

function entry(
  base: MeshAccessSnapshot,
  draft: MeshAccessPolicy,
  builder: WorkingGrant | null = null
): StoredAccessDraft {
  return { base, inventory: [], draft, builder, nextSession: 0 }
}

function editing(): WorkingGrant {
  const initial = newGrantForm("2".repeat(64))
  return { session: { id: 1, mode: "new", initial }, form: { ...initial, source: A } }
}

test("a late completion from an unmounted editor cannot touch a newer editor's draft", () => {
  const registry = new AccessDraftRegistry()
  const first = registry.claim()
  registry.write(first, entry(snapshot(4), { grants: [] }))
  registry.release(first)

  const second = registry.claim()
  registry.write(second, entry(snapshot(4), { grants: [{ id: G1, path: [B, A] }] }))

  registry.clear(first)
  registry.write(first, entry(snapshot(9), policy))
  registry.adoptOrphanSave(4, accessKey({ grants: [{ id: G1, path: [B, A] }] }))

  assert.equal(registry.read()?.base.revision, 4)
  assert.deepEqual(registry.read()?.draft.grants[0]?.path, [B, A])
})

test("unapplied builder edits keep an entry and the CAS base while the draft is clean", () => {
  const registry = new AccessDraftRegistry()
  const editor = registry.claim()
  registry.write(editor, entry(snapshot(7), policy, editing()))
  registry.release(editor)
  assert.equal(registry.hasDraft(), true)
  assert.equal(registry.read()?.builder?.form.source, A)
  assert.equal(registry.read()?.base.revision, 7)
})

test("an orphaned save clears only exactly-saved work on the same base without builder edits", () => {
  const sent: MeshAccessPolicy = { grants: [] }
  const sentKey = accessKey(sent)

  const plain = new AccessDraftRegistry()
  const owner = plain.claim()
  plain.write(owner, entry(snapshot(5), sent))
  plain.release(owner)
  plain.adoptOrphanSave(5, sentKey)
  assert.equal(plain.read(), null)

  const withBuilder = new AccessDraftRegistry()
  const editor = withBuilder.claim()
  withBuilder.write(editor, entry(snapshot(5), sent, editing()))
  withBuilder.release(editor)
  withBuilder.adoptOrphanSave(5, sentKey)
  assert.equal(withBuilder.read()?.builder?.form.source, A, "unapplied edits survive")

  const otherBase = new AccessDraftRegistry()
  const stale = otherBase.claim()
  otherBase.write(stale, entry(snapshot(6), sent))
  otherBase.release(stale)
  otherBase.adoptOrphanSave(5, sentKey)
  assert.equal(otherBase.read()?.base.revision, 6)

  const changed = new AccessDraftRegistry()
  const later = changed.claim()
  changed.write(later, entry(snapshot(5), policy))
  changed.release(later)
  changed.adoptOrphanSave(5, sentKey)
  assert.deepEqual(changed.read()?.draft, policy, "different work is kept on its old base")
})
