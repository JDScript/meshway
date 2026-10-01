import assert from "node:assert/strict"
import { test } from "node:test"

import type { PolicySnapshot, RoutingPolicy } from "@meshway/control-client"

import { zPolicySnapshot } from "../../../packages/control-client/src/generated/zod.gen.ts"
import { fromPolicy, policyKey, toPolicy, type PolicyDraft } from "./policy-model.ts"
import { newRuleForm, toRuleForm, type WorkingRule } from "./rule-form.ts"
import { RoutingDraftRegistry, type StoredRoutingDraft } from "./routing-draft-registry.ts"

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
    valid: true,
    diagnostics: [],
    network: { ipv4_prefix: "100.96.0.0/16", ipv6_prefix: "fd00::/64" },
  })
}

function entry(base: PolicySnapshot, draft: PolicyDraft, builder: WorkingRule | null = null): StoredRoutingDraft {
  return { base, nodes: [], draft, builder, nextKey: 0, nextSession: 0 }
}

function editing(draft: PolicyDraft): WorkingRule {
  const [rule] = draft.rules
  assert.ok(rule)
  const initial = toRuleForm(rule)
  return { session: { id: 1, mode: "edit", initial, position: 0 }, form: { ...initial, id: "renamed" } }
}

test("a late reload or save from an unmounted editor cannot clear a newer editor's draft", () => {
  const registry = new RoutingDraftRegistry()
  const first = registry.claim()
  const edited = { ...fromPolicy(policy), defaultAction: "direct" as const }
  registry.write(first, entry(snapshot(4), edited))
  registry.release(first)

  const second = registry.claim()
  const kept = registry.read()
  assert.ok(kept)
  registry.write(second, { ...kept, draft: { ...edited, rules: [] } })

  // The first editor's delayed completions only act while it owns the store.
  registry.clear(first)
  registry.write(first, entry(snapshot(9), fromPolicy(policy)))
  registry.adoptOrphanSave(4, policyKey(toPolicy({ ...edited, rules: [] })))

  assert.equal(registry.read()?.base.revision, 4)
  assert.equal(registry.read()?.draft.rules.length, 0)
})

test("unapplied builder edits keep an entry and the CAS base while the policy is clean", () => {
  const registry = new RoutingDraftRegistry()
  const editor = registry.claim()
  const base = snapshot(7)
  const draft = fromPolicy(base.policy)
  registry.write(editor, entry(base, draft, editing(draft)))
  registry.release(editor)
  assert.equal(registry.hasDraft(), true)
  assert.equal(registry.read()?.builder?.form.id, "renamed")
  assert.equal(registry.read()?.base.revision, 7)
})

test("an orphaned save clears only matching work without unapplied builder edits", () => {
  const sent = { ...fromPolicy(policy), defaultAction: "direct" as const }
  const sentKey = policyKey(toPolicy(sent))

  const plain = new RoutingDraftRegistry()
  plain.release(plain.claim())
  const owner = plain.claim()
  plain.write(owner, entry(snapshot(5), sent))
  plain.release(owner)
  plain.adoptOrphanSave(5, sentKey)
  assert.equal(plain.read(), null)

  const withBuilder = new RoutingDraftRegistry()
  const editor = withBuilder.claim()
  withBuilder.write(editor, entry(snapshot(5), sent, editing(sent)))
  withBuilder.release(editor)
  withBuilder.adoptOrphanSave(5, sentKey)
  assert.equal(withBuilder.read()?.builder?.form.id, "renamed", "unapplied edits survive")

  const otherBase = new RoutingDraftRegistry()
  const stale = otherBase.claim()
  otherBase.write(stale, entry(snapshot(6), sent))
  otherBase.release(stale)
  otherBase.adoptOrphanSave(5, sentKey)
  assert.equal(otherBase.read()?.base.revision, 6)
})

test("new rule forms start narrow, not as an accidental match-all", () => {
  const form = newRuleForm("new-0", "rule-1")
  assert.equal(form.sourceMode, "specific")
  assert.equal(form.targetMode, "specific")
})
