import assert from "node:assert/strict"
import { test } from "node:test"

import type { RoutingPolicy } from "@meshway/control-client"

// Parse with the generated runtime schema, exactly as the SDK does, so the
// test sees the SDK's real object key order rather than a hand-written one.
import { zRoutingPolicy } from "../../../packages/control-client/src/generated/zod.gen.ts"
import {
  fromPolicy,
  policyKey,
  removedRuleCount,
  ruleChange,
  samePolicy,
  toPolicy,
} from "./policy-model.ts"

// Written in the editor's order (id first); the schema orders keys
// alphabetically (action first).
const written: RoutingPolicy = {
  default_action: "reject",
  rules: [
    {
      id: "nyc-to-sha",
      source: { node_ids: ["nyc"], countries: [] },
      destination_scope: "mesh",
      destination_any: [{ kind: "node", value: "sha" }],
      action: { kind: "route", via: ["lax", "tyo"], exit_node_id: null },
    },
    {
      id: "us-to-cn",
      source: { node_ids: [], countries: ["US"] },
      destination_scope: "internet",
      destination_any: [
        { kind: "domain_suffix", value: "example.cn" },
        { kind: "cidr", value: "203.0.113.0/24" },
      ],
      action: { kind: "route", via: [], exit_node_id: "lax" },
    },
  ],
}

const parsed: RoutingPolicy = zRoutingPolicy.parse(JSON.parse(JSON.stringify(written)))

test("the SDK really orders keys differently from the editor", () => {
  assert.notEqual(JSON.stringify(parsed), JSON.stringify(toPolicy(fromPolicy(parsed))))
})

test("a freshly loaded policy is clean", () => {
  const draft = fromPolicy(parsed)
  assert.equal(samePolicy(toPolicy(draft), parsed), true)
  assert.equal(policyKey(toPolicy(draft)), policyKey(parsed))
  assert.deepEqual(
    draft.rules.map((rule) => ruleChange(rule, parsed)),
    ["unchanged", "unchanged"]
  )
  assert.equal(removedRuleCount(draft, parsed), 0)
})

test("identity ignores key order but not array order", () => {
  assert.equal(policyKey(parsed), policyKey(written))

  const [first, second] = written.rules
  assert.ok(first && second)
  const reordered: RoutingPolicy = { ...written, rules: [second, first] }
  assert.equal(samePolicy(reordered, written), false)

  const swappedVia: RoutingPolicy = {
    ...written,
    rules: [{ ...first, action: { ...first.action, via: ["tyo", "lax"] } }, second],
  }
  assert.equal(samePolicy(swappedVia, written), false)

  const swappedSelectors: RoutingPolicy = {
    ...written,
    rules: [first, { ...second, destination_any: [...second.destination_any].reverse() }],
  }
  assert.equal(samePolicy(swappedSelectors, written), false)
})

test("an edit marks only that rule as modified", () => {
  const draft = fromPolicy(parsed)
  const [first, second] = draft.rules
  assert.ok(first && second)
  const edited = { ...draft, rules: [first, { ...second, exitNodeId: "sha" }] }
  assert.equal(samePolicy(toPolicy(edited), parsed), false)
  assert.deepEqual(
    edited.rules.map((rule) => ruleChange(rule, parsed)),
    ["unchanged", "modified"]
  )
})
