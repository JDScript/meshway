import assert from "node:assert/strict"
import { test } from "node:test"

import type { PolicySnapshot, RoutingPolicy } from "@meshway/control-client"
import { zPolicySnapshot, zRoutingPolicy } from "@meshway/control-client/schemas"

import { newRuleForm, toRuleForm, type WorkingRule } from "./rule-form.ts"
import { RoutingDraftRegistry, type StoredRoutingDraft } from "./routing-draft-registry.ts"
import {
  applyDraft,
  editWorkspaceText,
  openWorkspace,
  routingBase,
  storeWorkspace,
  workspaceDraft,
  type RoutingWorkspace,
} from "./routing-workspace.ts"

const policy: RoutingPolicy = {
  default_action: "reject",
  rules: [
    {
      id: "nyc-to-sha",
      source: { node_ids: ["nyc"], countries: [] },
      destination_scope: "mesh",
      destination_any: [{ kind: "node", value: "sha" }],
      action: { kind: "route", via: ["lax"], exit_node_id: null },
    },
  ],
}

function snapshot(revision: number, value: RoutingPolicy = policy): PolicySnapshot {
  return zPolicySnapshot.parse({
    revision,
    policy: value,
    source_jsonc: null,
    valid: true,
    diagnostics: [],
    network: { ipv4_prefix: "100.96.0.0/16", ipv6_prefix: "fd00::/64" },
  })
}

const open = (base: PolicySnapshot) => openWorkspace(routingBase(base), zRoutingPolicy)

function direct(workspace: RoutingWorkspace): RoutingWorkspace {
  const result = applyDraft(workspace, { ...workspaceDraft(workspace), defaultAction: "direct" }, zRoutingPolicy)
  assert.ok(result.ok)
  return result.workspace
}

function entry(base: PolicySnapshot, workspace: RoutingWorkspace, builder: WorkingRule | null = null): StoredRoutingDraft {
  return { base, nodes: [], workspace: storeWorkspace(workspace), builder, nextKey: 0, nextSession: 0, view: "visual" }
}

function editing(workspace: RoutingWorkspace): WorkingRule {
  const [rule] = workspaceDraft(workspace).rules
  assert.ok(rule)
  const initial = toRuleForm(rule)
  return { session: { id: 1, mode: "edit", initial, position: 0 }, form: { ...initial, id: "renamed" } }
}

function saved(revision: number, value: RoutingPolicy) {
  const next = snapshot(revision, value)
  return { snapshot: next, base: routingBase(next) }
}

test("a late reload or save from an unmounted editor cannot clear a newer editor's draft", () => {
  const registry = new RoutingDraftRegistry()
  const first = registry.claim()
  const edited = direct(open(snapshot(4)))
  registry.write(first, entry(snapshot(4), edited))
  registry.release(first)

  const second = registry.claim()
  const kept = registry.read()
  assert.ok(kept)
  registry.write(second, { ...kept, view: "jsonc" })

  registry.clear(first)
  registry.write(first, entry(snapshot(9), open(snapshot(9))))
  registry.adoptOrphanSave(4, edited.document.seq, saved(5, edited.document.value), zRoutingPolicy)

  assert.equal(registry.read()?.base.revision, 4)
  assert.equal(registry.read()?.view, "jsonc")
})

test("unapplied builder edits keep an entry and the CAS base while the policy is clean", () => {
  const registry = new RoutingDraftRegistry()
  const editor = registry.claim()
  const base = snapshot(7)
  const workspace = open(base)
  registry.write(editor, entry(base, workspace, editing(workspace)))
  registry.release(editor)
  assert.equal(registry.hasDraft(), true)
  assert.equal(registry.read()?.builder?.form.id, "renamed")
  assert.equal(registry.read()?.base.revision, 7)
})

test("an orphaned save clears only the exact work that was sent", () => {
  const sent = direct(open(snapshot(5)))

  const plain = new RoutingDraftRegistry()
  const owner = plain.claim()
  plain.write(owner, entry(snapshot(5), sent))
  plain.release(owner)
  plain.adoptOrphanSave(5, sent.document.seq, saved(6, sent.document.value), zRoutingPolicy)
  assert.equal(plain.read(), null)

  const withBuilder = new RoutingDraftRegistry()
  const editor = withBuilder.claim()
  withBuilder.write(editor, entry(snapshot(5), sent, editing(sent)))
  withBuilder.release(editor)
  withBuilder.adoptOrphanSave(5, sent.document.seq, saved(6, sent.document.value), zRoutingPolicy)
  assert.equal(withBuilder.read()?.builder?.form.id, "renamed", "unapplied edits survive")

  const later = new RoutingDraftRegistry()
  const typing = later.claim()
  const edited = editWorkspaceText(sent, sent.document.text.replace('"direct"', '"reject"'), zRoutingPolicy)
  later.write(typing, entry(snapshot(5), edited))
  later.release(typing)
  later.adoptOrphanSave(5, sent.document.seq, saved(6, sent.document.value), zRoutingPolicy)
  assert.equal(later.read()?.base.revision, 5, "edits made after sending stay on their base")
})

test("comments the controller did not store survive an orphaned save on the new base", () => {
  const commented = editWorkspaceText(
    direct(open(snapshot(5))),
    `// keep me\n${direct(open(snapshot(5))).document.text}`,
    zRoutingPolicy
  )
  const registry = new RoutingDraftRegistry()
  const owner = registry.claim()
  registry.write(owner, entry(snapshot(5), commented))
  registry.release(owner)
  registry.adoptOrphanSave(5, commented.document.seq, saved(6, commented.document.value), zRoutingPolicy)
  const kept = registry.read()
  assert.ok(kept)
  assert.equal(kept.base.revision, 6)
  assert.ok(kept.workspace.document.text.startsWith("// keep me"))
})

test("new rule forms start narrow, not as an accidental match-all", () => {
  const form = newRuleForm("new-0", "rule-1")
  assert.equal(form.sourceMode, "specific")
  assert.equal(form.targetMode, "specific")
})
