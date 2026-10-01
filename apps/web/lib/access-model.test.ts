import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionNode, MeshAccessPolicy, MeshAccessRule, TagSnapshot } from "@meshway/control-client"

import {
  accessSaveFailure,
  removedRuleCount,
  ruleChange,
  shadowedBy,
  suggestRuleId,
  tagMembers,
  validateAccess,
} from "./access-model.ts"
import { toAccessRule, updateRuleForm, newRuleForm, validateRuleForm } from "./access-rule-form.ts"

const hex = (seed: string) => seed.repeat(64).slice(0, 64)
const NYC = hex("a")
const LAX = hex("b")
const SHA = hex("c")
const OLD = hex("d")
const NEW = hex("e")

function entry(id: string, name: string, options: { revoked?: boolean; key?: boolean } = {}): ConnectionNode {
  return {
    acknowledged_at: null,
    acknowledged_digest: null,
    acknowledged_revision: 0,
    capabilities: [],
    public_key: options.key === false ? null : hex("f"),
    node: {
      acknowledged_revision: 0,
      agent_version: "0.0.1",
      configuration_revision: 1,
      country: null,
      id,
      last_seen: "2026-10-01T00:00:00Z",
      mesh_ipv4: "100.96.0.1",
      mesh_ipv6: "fd00::1",
      name,
      registered_at: "2026-10-01T00:00:00Z",
      revoked_at: options.revoked ? "2026-10-01T01:00:00Z" : null,
    },
  }
}

const inventory = [entry(NYC, "nyc"), entry(LAX, "lax"), entry(SHA, "sha"), entry(OLD, "old", { revoked: true }), entry(NEW, "fresh", { key: false })]

const tags: TagSnapshot = {
  revision: 7,
  tags: [
    { tag: "tag:edge", description: "边缘" },
    { tag: "tag:empty", description: "" },
  ],
  assignments: [
    { node_id: NYC, tags: ["tag:edge"] },
    { node_id: SHA, tags: ["tag:edge"] },
    { node_id: OLD, tags: ["tag:edge"] },
  ],
}

const rule = (patch: Partial<MeshAccessRule>): MeshAccessRule => ({
  id: "r",
  source: ["*"],
  destination: ["*"],
  action: "allow",
  bidirectional: false,
  ...patch,
})

test("tag members are active nodes only", () => {
  assert.deepEqual(
    tagMembers(tags, "tag:edge", inventory).map((member) => member.node.name),
    ["nyc", "sha"]
  )
  assert.deepEqual(tagMembers(tags, "tag:empty", inventory), [])
})

test("validation flags broken references with request-body field paths", () => {
  const policy: MeshAccessPolicy = {
    default_action: "reject",
    rules: [
      rule({ id: "ok", source: ["tag:edge"], destination: [`node:${NEW}`] }),
      rule({ id: "bad", source: [`node:${OLD}`, "tag:missing"], destination: [`node:${hex("9")}`] }),
      rule({ id: "bad", source: ["*", "tag:edge"], destination: [] }),
    ],
  }
  const issues = validateAccess(policy, inventory, tags)
  assert.ok(!issues.some((issue) => issue.ruleIndex === 0), "keyless nodes and empty-capable tags are valid subjects")
  assert.deepEqual(
    issues.filter((issue) => issue.ruleIndex === 1 && !issue.field.endsWith(".id")).map((issue) => issue.field),
    ["rules[1].source[0]", "rules[1].source[1]", "rules[1].destination[0]"]
  )
  const third = issues.filter((issue) => issue.ruleIndex === 2).map((issue) => issue.field)
  assert.ok(third.includes("rules[2].id"), "duplicate ID")
  assert.ok(third.includes("rules[2].source[0]"), "* must stand alone")
  assert.ok(third.includes("rules[2].destination"), "empty list")
})

test("changes are judged by rule ID against the saved policy", () => {
  const base: MeshAccessPolicy = { default_action: "reject", rules: [rule({ id: "a" }), rule({ id: "b" })] }
  assert.equal(ruleChange(rule({ id: "a" }), base), "unchanged")
  assert.equal(ruleChange(rule({ id: "a", bidirectional: true }), base), "modified")
  assert.equal(ruleChange(rule({ id: "c" }), base), "new")
  assert.equal(removedRuleCount({ ...base, rules: [rule({ id: "b" })] }, base), 1)
})

test("shadowing is judged on selector text only, both directions of bidirectional rules", () => {
  const policy: MeshAccessPolicy = {
    default_action: "reject",
    rules: [
      rule({ id: "edge-core", source: ["tag:edge"], destination: ["tag:core"], bidirectional: true }),
      rule({ id: "core-edge", source: ["tag:core"], destination: ["tag:edge"] }),
      rule({ id: "nyc-core", source: [`node:${NYC}`], destination: ["tag:core"] }),
      rule({ id: "all", source: ["*"], destination: ["*"] }),
      rule({ id: "after-all", source: ["tag:x"], destination: ["tag:y"] }),
    ],
  }
  assert.equal(shadowedBy(policy, 1), 0, "the reverse half of a bidirectional rule covers it")
  assert.equal(shadowedBy(policy, 2), null, "tag membership is never expanded")
  assert.equal(shadowedBy(policy, 4), 3)
})

test("rule IDs are readable suggestions, unique and stable once typed", () => {
  const taken = ["allow-edge-to-all"]
  assert.equal(suggestRuleId(rule({ source: ["tag:edge"], destination: ["*"] }), taken, inventory), "allow-edge-to-all-2")
  assert.equal(suggestRuleId(rule({ action: "reject", source: [`node:${NYC}`], destination: [`node:${SHA}`] }), [], inventory), "reject-nyc-to-sha")
  let form = updateRuleForm(newRuleForm(), { source: ["tag:edge"], destination: [`node:${SHA}`] }, [], inventory)
  assert.equal(form.id, "allow-edge-to-sha")
  form = updateRuleForm(form, { id: "my-rule" }, [], inventory)
  form = updateRuleForm(form, { destination: ["*"] }, [], inventory)
  assert.equal(form.id, "my-rule")
  assert.equal(toAccessRule(form).bidirectional, true, "new rules start bidirectional")
  assert.deepEqual(validateRuleForm(form, [rule({ id: "my-rule" })], inventory, tags).map((issue) => issue.field), ["id"])
})

test("save failures never carry a replacement and describe the cause", () => {
  assert.equal(accessSaveFailure(409, 4, "x").status, "conflict")
  assert.match(accessSaveFailure(422, 4, "unknown tag").message, /unknown tag/)
  assert.equal(accessSaveFailure(null, 4, "offline").status, "error")
})
