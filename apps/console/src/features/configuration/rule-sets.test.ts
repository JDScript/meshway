import assert from "node:assert/strict"
import { test } from "node:test"
import type { RuleSet, RuleSetView } from "@meshway/control-client"

import {
  durationMs,
  inlineOf,
  isSourceUrl,
  preparationOf,
  refreshable,
  ruleSetProblems,
  sameRuleSets,
  usable,
  writtenRuleSet,
} from "./rule-sets.ts"

const remote: RuleSet = {
  type: "remote",
  url: "https://example.com/rules/cn.json",
  format: "sing-box-source",
}

const view = (fields: Partial<RuleSetView>): RuleSetView => ({
  key: "cn",
  configurationRevision: 3,
  ...fields,
})

const ARTIFACT = {
  id: "33333333-3333-4333-8333-333333333333",
  digest: "sha256:abc",
  entries: 12,
  createdAt: "2026-10-03T10:00:00Z",
}

test("durations take ms, s, m and h", () => {
  assert.equal(durationMs("24h"), 86_400_000)
  assert.equal(durationMs("1440m"), 86_400_000)
  assert.equal(durationMs("500ms"), 500)
  assert.equal(durationMs("0h"), null)
  assert.equal(durationMs("1d"), null)
  assert.equal(durationMs("24 h"), null)
})

test("sources are HTTPS without credentials or a fragment", () => {
  assert.equal(isSourceUrl("https://example.com/a.json"), true)
  assert.equal(isSourceUrl("https://example.com:8443/a.json?x=1"), true)
  assert.equal(isSourceUrl("http://example.com/a.json"), false)
  assert.equal(isSourceUrl("https://user:pw@example.com/a.json"), false)
  assert.equal(isSourceUrl("https://example.com/a.json#x"), false)
  assert.equal(isSourceUrl("https://exa mple.com/"), false)
  assert.equal(isSourceUrl(""), false)
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
  assert.deepEqual(inlineOf(["corp.example", "10.0.0.0/8"]), {
    type: "inline",
    domainSuffix: ["corp.example"],
    ipCIDR: ["10.0.0.0/8"],
  })
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

test("preparation: saved unreferenced, refreshed, then referenced", () => {
  assert.equal(
    preparationOf({ type: "inline" }, undefined, undefined),
    "inline"
  )
  assert.equal(preparationOf(remote, undefined, undefined), "unsaved")
  assert.equal(
    preparationOf(
      { ...remote, url: "https://example.com/b" },
      remote,
      view({})
    ),
    "changed"
  )
  assert.equal(preparationOf(remote, remote, undefined), "unread")
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
  // A missing set is never treated as empty: it cannot be used until refreshed.
  assert.equal(usable("missing"), false)
  assert.equal(usable("unsaved"), false)
  assert.equal(usable("failing"), true)
  assert.equal(refreshable("unsaved"), false)
  assert.equal(refreshable("changed"), false)
  assert.equal(refreshable("missing"), true)
})
