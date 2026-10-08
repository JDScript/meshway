import assert from "node:assert/strict"
import type { ConfigurationView } from "@meshway/control-client"
import { test } from "vitest"

import {
  draftConfiguration,
  draftRules,
  ruleText,
  targetOf,
} from "@/lib/configuration/evaluation"
import { withValue } from "@/lib/configuration/operations"

const LAX = "a119c88f-9d22-4bdb-8ec2-11fd57d1606d"
const NODES = [{ id: LAX, hostname: "meshway-lax" }]

const VIEW: ConfigurationView = {
  access: [{ remark: "lab ping", src: ["*"], dst: ["*"], ip: ["icmp:*"] }],
  addressPools: { ipv4: ["100.64.0.0/10"], ipv6: [] },
  defaults: { access: "deny", topology: "allow", dialers: {} },
  dns: { domain: "a.example" },
  groups: {},
  nodes: {},
  routing: {
    egress: {
      rules: [
        {
          match: { domainSuffix: ["telegram.org"] },
          action: { type: "exit", nodes: [LAX] },
        },
      ],
      profiles: {
        work: { rules: [], fallback: { type: "direct" } },
      },
    },
  },
  ruleSets: {},
  tagsOwners: {},
  topology: [],
}

const name = (id: string) => (id === LAX ? "meshway-lax" : id)

test("a destination as typed: a hostname, an IP address or a domain", () => {
  assert.deepEqual(targetOf(" Meshway-LAX ", NODES), {
    type: "node",
    nodeId: LAX,
  })
  assert.deepEqual(targetOf("100.64.0.3", NODES), {
    type: "address",
    address: "100.64.0.3",
  })
  assert.deepEqual(targetOf("2001:db8::1", NODES), {
    type: "address",
    address: "2001:db8::1",
  })
  assert.deepEqual(targetOf("web.telegram.org", NODES), {
    type: "domain",
    name: "web.telegram.org",
  })
  // Neither a node, an address nor a name with a dot.
  assert.equal(targetOf("laptop", NODES), null)
  assert.equal(targetOf("https://example.com/", NODES), null)
  assert.equal(targetOf("  ", NODES), null)
})

test("the draft is asked as a change at its base revision", () => {
  assert.deepEqual(draftConfiguration(4, { kind: "document", text: "{}" }), {
    type: "document",
    expectedRevision: 4,
    sourceJsonc: "{}",
  })
  const ops = withValue([], VIEW, "access", [])
  assert.deepEqual(
    draftConfiguration(4, { kind: "operations", base: VIEW, ops }),
    { type: "operations", expectedRevision: 4, ops }
  )
})

test("a rule the controller points at is named as the forms number it, with its remark", () => {
  assert.deepEqual(ruleText("/access/0", VIEW, name), {
    name: "访问规则 1",
    remark: "lab ping",
  })
  assert.deepEqual(ruleText("/routing/egress/rules/0", VIEW, name), {
    name: "公共出口规则 1",
    remark: null,
  })
  assert.deepEqual(
    ruleText("/routing/egress/profiles/work/fallback", VIEW, name),
    { name: "方案「work」的兜底", remark: null }
  )
  assert.equal(
    ruleText(`/routing/egress/profiles/global:${LAX}/fallback`, VIEW, name)
      .name,
    "方案「全局经 meshway-lax」的兜底"
  )
  assert.equal(ruleText("/topology/2", VIEW, name).name, "组网规则 3")
  // A pointer the console does not know stays as written.
  assert.equal(ruleText("/dns/servers/0", VIEW, name).name, "/dns/servers/0")
})

test("the draft's rules: the source as typed, or the forms' values over their base", () => {
  const ops = withValue([], VIEW, "access", [
    { remark: "new", src: ["*"], dst: ["*"], ip: ["tcp:443"] },
  ])
  const rules = draftRules({ kind: "operations", base: VIEW, ops })
  assert.equal(ruleText("/access/0", rules, name).remark, "new")
  const typed = draftRules({
    kind: "document",
    text: '{ "access": [ { "remark": "typed", /* c */ "src": [] } ] }',
  })
  assert.equal(ruleText("/access/0", typed, name).remark, "typed")
  assert.equal(draftRules({ kind: "document", text: "{" }), null)
})
