import assert from "node:assert/strict"
import type { RuleSet, RuleSetView } from "@meshway/control-client"
import { test } from "vitest"

import {
  effectiveProviders,
  inlineOf,
  inlineValues,
  preparationOf,
  refreshable,
  ruleSetProblems,
  sameRuleSets,
  writtenRuleSet,
} from "@/lib/configuration/rule-sets"

const remote: RuleSet = {
  type: "remote",
  url: "https://example.com/rules/cn.json",
  format: "sing-box-source",
}

const view = (fields: Partial<RuleSetView>): RuleSetView => ({
  key: "cn",
  configurationRevision: 3,
  implicit: false,
  provider: null,
  source: null,
  conversion: null,
  held: null,
  ...fields,
})

const ARTIFACT = {
  digest: `sha256:${"ab".repeat(32)}`,
  format: 1,
  bytes: 584,
  entries: 12,
  createdAt: "2026-10-03T10:00:00Z",
}

const codesOf = (set: RuleSet) =>
  ruleSetProblems({ cn: set }).map((problem) => problem.code)

test("intervals take ms, s, m and h", () => {
  for (const interval of ["24h", "1440m", "90s", "500ms"])
    assert.deepEqual(codesOf({ ...remote, updateInterval: interval }), [])
  for (const interval of ["0h", "1d", "24 h"])
    assert.deepEqual(codesOf({ ...remote, updateInterval: interval }), [
      "duration",
    ])
})

test("sources are HTTPS without credentials or a fragment", () => {
  for (const url of [
    "https://example.com/a.json",
    "https://example.com:8443/a.json?x=1",
  ])
    assert.deepEqual(codesOf({ ...remote, url }), [])
  for (const url of [
    "http://example.com/a.json",
    "https://user:pw@example.com/a.json",
    "https://example.com/a.json#x",
    "https://exa mple.com/",
    "",
  ])
    assert.deepEqual(codesOf({ ...remote, url }), ["url"])
})

test("the default interval and the order of inline values make no change", () => {
  assert.equal(
    sameRuleSets(
      { cn: remote },
      { cn: { ...remote, updateInterval: "1440m" } }
    ),
    true
  )
  assert.equal(
    sameRuleSets({ cn: remote }, { cn: { ...remote, updateInterval: "1h" } }),
    false
  )
  assert.equal(
    sameRuleSets(
      { o: { type: "inline", ipCIDR: [], domainSuffix: ["b", "a"] } },
      { o: { type: "inline", domainSuffix: ["a", "b"] } }
    ),
    true
  )
  assert.deepEqual(writtenRuleSet({ ...remote, updateInterval: "24h" }), remote)
  assert.deepEqual(
    inlineOf(["corp.example", "=login.partner.example", "10.0.0.0/8"]),
    {
      type: "inline",
      domainSuffix: ["corp.example"],
      domain: ["login.partner.example"],
      ipCIDR: ["10.0.0.0/8"],
    }
  )
  assert.equal(
    sameRuleSets(
      { o: { type: "inline", domainSuffix: ["a.example"] } },
      { o: { type: "inline", domain: ["a.example"] } }
    ),
    false
  )
})

test("names matched only below a name are kept, written with a leading dot", () => {
  const set: RuleSet = {
    type: "inline",
    domainSuffix: ["corp.example"],
    domainSubdomain: ["cdn.example"],
  }
  assert.ok(set.type === "inline")
  assert.deepEqual(inlineValues(set), ["corp.example", ".cdn.example"])
  assert.deepEqual(inlineOf(inlineValues(set)), set)
  assert.deepEqual(writtenRuleSet(set), set)
  assert.equal(
    sameRuleSets({ a: set }, { a: { ...set, domainSubdomain: [] } }),
    false
  )
})

test("a set's name cannot hold ':', which names a catalog set", () => {
  assert.deepEqual(
    ruleSetProblems({ "geo:cn": { type: "inline", ipCIDR: ["10.0.0.0/8"] } }),
    [{ path: "/ruleSets/geo:cn", code: "set_key" }]
  )
})

test("the providers a reference can name: built-ins not turned off, then the configuration's", () => {
  assert.deepEqual(effectiveProviders({}), ["geosite", "geoip"])
  assert.deepEqual(
    effectiveProviders({
      geoip: null,
      corp: {
        catalog: "https://rules.corp.example/index.json",
        collection: "corp",
        publicKeys: [],
      },
      old: null,
    }),
    ["geosite", "corp"]
  )
})

test("problems sit at the controller's pointers", () => {
  assert.deepEqual(
    ruleSetProblems({
      empty: { type: "inline" },
      bad: { type: "inline", ipCIDR: ["10.0.0.1/8"] },
      plain: { ...remote, url: "http://x.example/", updateInterval: "1d" },
    }),
    [
      { path: "/ruleSets/empty", code: "empty" },
      { path: "/ruleSets/bad", code: "destination" },
      { path: "/ruleSets/plain/url", code: "url" },
      { path: "/ruleSets/plain/updateInterval", code: "duration" },
    ]
  )
})

test("preparation: a new or replaced source is prepared with the draft", () => {
  assert.equal(
    preparationOf({ type: "inline" }, undefined, undefined),
    "inline"
  )
  assert.equal(preparationOf(remote, undefined, undefined), "new")
  assert.equal(
    preparationOf(
      { ...remote, url: "https://example.com/b" },
      remote,
      view({})
    ),
    "replaced"
  )
  assert.equal(
    preparationOf({ ...remote, format: "mihomo-domain" }, remote, view({})),
    "replaced"
  )
  assert.equal(preparationOf(remote, { type: "inline" }, view({})), "replaced")
  // Only the interval changed: the same content, its status still holds.
  assert.equal(
    preparationOf(
      { ...remote, updateInterval: "1h" },
      remote,
      view({ artifact: ARTIFACT })
    ),
    "ready"
  )
  assert.equal(preparationOf(remote, remote, undefined), "unread")
  // A set without content is never treated as empty.
  assert.equal(
    preparationOf(remote, remote, view({ problem: "not_ready" })),
    "missing"
  )
  assert.equal(
    preparationOf(
      remote,
      remote,
      view({ artifact: ARTIFACT, problem: "download_failed" })
    ),
    "failing"
  )
  assert.equal(
    preparationOf(remote, remote, view({ artifact: ARTIFACT })),
    "ready"
  )
  assert.equal(refreshable("new"), false)
  assert.equal(refreshable("replaced"), false)
  assert.equal(refreshable("missing"), true)
  assert.equal(refreshable("failing"), true)
})
