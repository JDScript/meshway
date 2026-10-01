import assert from "node:assert/strict"
import { test } from "node:test"

import type { PolicySnapshot, RoutingPolicy } from "@meshway/control-client"
import { zPolicySnapshot, zRoutingPolicy } from "@meshway/control-client/schemas"

import { formatJsonc } from "./jsonc.ts"
import { isValid } from "./policy-document.ts"
import { removedRuleCount, ruleChange, validateDraft } from "./policy-model.ts"
import {
  applyDraft,
  discardWorkspaceText,
  editWorkspaceText,
  localIssueProblems,
  openWorkspace,
  restoreWorkspace,
  revertWorkspace,
  routingBase,
  settleWorkspaceSave,
  storeWorkspace,
  workspaceDraft,
} from "./routing-workspace.ts"

const rule = (id: string, node: string): RoutingPolicy["rules"][number] => ({
  id,
  source: { node_ids: [], countries: [] },
  destination_scope: "mesh",
  destination_any: [{ kind: "node", value: node }],
  action: { kind: "route", via: [], exit_node_id: null },
})

const policy: RoutingPolicy = { default_action: "reject", rules: [rule("a", "nyc"), rule("b", "sha")] }

const snapshot = (revision: number, value: RoutingPolicy = policy): PolicySnapshot =>
  zPolicySnapshot.parse({
    revision,
    policy: value,
    source_jsonc: null,
    valid: true,
    diagnostics: [],
    network: { ipv4_prefix: "100.96.0.0/16", ipv6_prefix: "fd00::/64" },
  })

const schema = zRoutingPolicy
const open = () => openWorkspace(routingBase(snapshot(3)), schema)

test("a reorder in the visual editor keeps rule identity and comments", () => {
  const commented = editWorkspaceText(open(), `// why b first\n${open().document.text}`, schema)
  const draft = workspaceDraft(commented)
  const result = applyDraft(commented, { ...draft, rules: [...draft.rules].reverse() }, schema)
  assert.ok(result.ok)
  const moved = workspaceDraft(result.workspace)
  assert.deepEqual(
    moved.rules.map((candidate) => candidate.id),
    ["b", "a"]
  )
  assert.deepEqual(
    moved.rules.map((candidate) => ruleChange(candidate, policy)),
    ["unchanged", "unchanged"]
  )
  assert.ok(result.workspace.document.text.startsWith("// why b first"))
})

test("text edits keep keys by rule id, so renames in place and new rules are told apart", () => {
  const workspace = open()
  const text = workspace.document.text.replace('"value": "sha"', '"value": "lax"')
  const edited = editWorkspaceText(workspace, text, schema)
  const draft = workspaceDraft(edited)
  assert.deepEqual(
    draft.rules.map((candidate) => ruleChange(candidate, policy)),
    ["unchanged", "modified"]
  )
  const second = edited.document.value.rules[1]
  assert.ok(second)
  const removed = editWorkspaceText(edited, formatJsonc({ default_action: "reject", rules: [second] }), schema)
  assert.ok(isValid(removed.document), removed.document.text)
  assert.equal(removedRuleCount(workspaceDraft(removed), policy), 1)
  const [kept] = workspaceDraft(removed).rules
  assert.ok(kept)
  assert.equal(ruleChange(kept, policy), "modified", "b keeps its identity at its new position")
})

test("an invalid buffer blocks visual edits and keeps the visual model", () => {
  const broken = editWorkspaceText(open(), `{ "default_action": `, schema)
  assert.equal(isValid(broken.document), false)
  assert.equal(workspaceDraft(broken).rules.length, 2)
  const result = applyDraft(broken, { ...workspaceDraft(broken), defaultAction: "direct" }, schema)
  assert.equal(result.ok, false)
  const discarded = discardWorkspaceText(broken, schema)
  assert.ok(isValid(discarded.document))
})

test("editor issues are placed under the rule they belong to", () => {
  const workspace = open()
  const draft = workspaceDraft(workspace)
  const result = applyDraft(workspace, { ...draft, rules: draft.rules.map((candidate) => ({ ...candidate, id: "same" })) }, schema)
  assert.ok(result.ok)
  const problems = localIssueProblems(result.workspace, validateDraft(workspaceDraft(result.workspace)))
  assert.equal(problems.length, 2)
  assert.deepEqual(problems[1]?.path, ["rules", 1, "id"])
  assert.equal(result.workspace.document.text.slice(problems[1]?.from, problems[1]?.to), '"same"')
})

test("save, revert and restore keep the base, keys and text consistent", () => {
  const edited = editWorkspaceText(open(), `// c\n${open().document.text.replace('"reject"', '"direct"')}`, schema)
  const settled = settleWorkspaceSave(edited, routingBase(snapshot(4, edited.document.value)), edited.document.seq, schema)
  assert.equal(settled.document.base.revision, 4)
  assert.deepEqual(settled.keys, ["saved-0", "saved-1"])
  assert.ok(settled.document.text.startsWith("// c"))
  const reverted = revertWorkspace(edited, schema)
  assert.equal(reverted.document.value.default_action, "reject")
  const restored = restoreWorkspace(storeWorkspace(edited), schema)
  assert.equal(restored.document.text, edited.document.text)
  assert.deepEqual(restored.keys, edited.keys)
})
