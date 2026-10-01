import assert from "node:assert/strict"
import { test } from "node:test"

import type { MeshAccessPolicy, MeshAccessSnapshot, TagSnapshot } from "@meshway/control-client"
import { zMeshAccessPolicy, zMeshAccessSnapshot } from "@meshway/control-client/schemas"

import { AccessDraftRegistry, accessBase, type StoredAccessDraft } from "./access-draft-registry.ts"
import { newRuleForm, type WorkingAccessRule } from "./access-rule-form.ts"
import { applyVisual, editText, openDocument, storeDocument, type PolicyDocument } from "./policy-document.ts"

const A = "a".repeat(64)
const B = "b".repeat(64)

// The migrated demo shape: endpoint rules, both directions on one rule.
const policy: MeshAccessPolicy = {
  default_action: "reject",
  rules: [{ id: "nyc-sha", source: [`node:${A}`], destination: [`node:${B}`], action: "allow", bidirectional: true }],
}

const tags: TagSnapshot = { revision: 4, tags: [], assignments: [] }

function snapshot(revision: number, value: MeshAccessPolicy = policy, source: string | null = null): MeshAccessSnapshot {
  return zMeshAccessSnapshot.parse({ revision, policy: value, source_jsonc: source, valid: true, diagnostics: [] })
}

const open = (base: MeshAccessSnapshot) => openDocument(accessBase(base), zMeshAccessPolicy)

function allowAll(document: PolicyDocument<MeshAccessPolicy>): PolicyDocument<MeshAccessPolicy> {
  const next: MeshAccessPolicy = { ...document.value, default_action: "allow" }
  const result = applyVisual(document, next, zMeshAccessPolicy)
  assert.ok(result.ok)
  return result.document
}

function entry(
  base: MeshAccessSnapshot,
  document: PolicyDocument<MeshAccessPolicy>,
  builder: WorkingAccessRule | null = null
): StoredAccessDraft {
  return { base, inventory: [], tags, document: storeDocument(document), builder, nextSession: 0, view: "visual" }
}

function editing(): WorkingAccessRule {
  const initial = newRuleForm()
  return { session: { id: 1, mode: "new", position: 1, initial }, form: { ...initial, source: ["*"] } }
}

test("the base keeps the stored JSONC source, so saved comments are not unsaved work", () => {
  const source = `// demo rules\n${JSON.stringify(policy)}`
  const document = open(snapshot(4, policy, source))
  assert.equal(document.text, source)
})

test("a late save from an unmounted editor cannot clear a newer editor's draft", () => {
  const registry = new AccessDraftRegistry()
  const first = registry.claim()
  const edited = allowAll(open(snapshot(4)))
  registry.write(first, entry(snapshot(4), edited))
  registry.release(first)

  const second = registry.claim()
  const kept = registry.read()
  assert.ok(kept)
  registry.write(second, { ...kept, view: "jsonc" })

  registry.clear(first)
  registry.write(first, entry(snapshot(9), open(snapshot(9))))
  registry.adoptOrphanSave(4, edited.seq, snapshot(5, edited.value), zMeshAccessPolicy)
  assert.equal(registry.read()?.base.revision, 4)
  assert.equal(registry.read()?.view, "jsonc")
})

test("an orphaned save clears only the exact work that was sent", () => {
  const sent = allowAll(open(snapshot(5)))

  const plain = new AccessDraftRegistry()
  const owner = plain.claim()
  plain.write(owner, entry(snapshot(5), sent))
  plain.release(owner)
  plain.adoptOrphanSave(5, sent.seq, snapshot(6, sent.value, sent.text), zMeshAccessPolicy)
  assert.equal(plain.read(), null)

  const withBuilder = new AccessDraftRegistry()
  const editor = withBuilder.claim()
  withBuilder.write(editor, entry(snapshot(5), sent, editing()))
  withBuilder.release(editor)
  withBuilder.adoptOrphanSave(5, sent.seq, snapshot(6, sent.value, sent.text), zMeshAccessPolicy)
  assert.deepEqual(withBuilder.read()?.builder?.form.source, ["*"], "unapplied builder edits survive")

  const later = new AccessDraftRegistry()
  const typing = later.claim()
  const edited = editText(sent, `// more\n${sent.text}`, zMeshAccessPolicy)
  later.write(typing, entry(snapshot(5), edited))
  later.release(typing)
  later.adoptOrphanSave(5, sent.seq, snapshot(6, sent.value, sent.text), zMeshAccessPolicy)
  assert.equal(later.read()?.base.revision, 5, "edits made after sending stay on their base")
})

test("an orphaned save the controller stored without the comments keeps them on the new base", () => {
  const commented = editText(open(snapshot(5)), `// keep\n${open(snapshot(5)).text}`, zMeshAccessPolicy)
  const registry = new AccessDraftRegistry()
  const owner = registry.claim()
  registry.write(owner, entry(snapshot(5), commented))
  registry.release(owner)
  registry.adoptOrphanSave(5, commented.seq, snapshot(6, commented.value, null), zMeshAccessPolicy)
  const kept = registry.read()
  assert.ok(kept)
  assert.equal(kept.base.revision, 6)
  assert.ok(kept.document.text.startsWith("// keep"))
})
