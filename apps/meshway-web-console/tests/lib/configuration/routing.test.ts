import assert from "node:assert/strict"
import type { RoutingConfiguration } from "@meshway/control-client"
import { test } from "vitest"

import {
  badDestination,
  matchOf,
  matchValues,
  moved,
  profileChoices,
  referencedSets,
  renameProfile,
  renameSetReferences,
  routingProblems,
  sameRouting,
  writtenRouting,
} from "@/lib/configuration/routing"

const A = "11111111-1111-4111-8111-111111111111"
const B = "22222222-2222-4222-8222-222222222222"

test("rules move by one place and stay put at the ends", () => {
  assert.deepEqual(moved(["a", "b", "c"], 0, 1), ["b", "a", "c"])
  assert.deepEqual(moved(["a", "b", "c"], 2, -1), ["a", "c", "b"])
  assert.deepEqual(moved(["a", "b", "c"], 0, -1), ["a", "b", "c"])
  assert.deepEqual(moved(["a", "b", "c"], 2, 1), ["a", "b", "c"])
})

test("a match is one set of chips: domains, prefixes and rule sets", () => {
  const match = {
    domainSuffix: ["corp.example"],
    ipCIDR: ["10.20.0.0/16"],
    ruleSets: ["office"],
  }
  assert.deepEqual(matchValues(match), [
    "corp.example",
    "10.20.0.0/16",
    "set:office",
  ])
  assert.deepEqual(matchOf(matchValues(match)), match)
  assert.deepEqual(matchOf(["fd00::/8"]), { ipCIDR: ["fd00::/8"] })
  assert.deepEqual(matchOf([]), {})
})

test("destinations are checked as the controller reads them", () => {
  assert.equal(badDestination("10.20.0.0/16"), false)
  assert.equal(badDestination("10.20.0.1/16"), true)
  assert.equal(badDestination("example.com"), false)
  assert.equal(badDestination("not a domain"), true)
  assert.equal(badDestination("set:"), true)
})

test("defaults and set order do not make a change; priority order does", () => {
  const base: RoutingConfiguration = {
    mesh: [{ remark: "", src: ["tag:a", "tag:b"], dst: ["*"], via: [] }],
    egress: {
      rules: [
        {
          remark: "",
          match: { domainSuffix: ["b.example", "a.example"], ipCIDR: [] },
          action: { type: "exit", nodes: [`node:${A}`, `node:${B}`], via: [] },
        },
      ],
      profiles: {},
      assignments: [
        { target: ["*"], allowed: ["mesh"], default: "mesh", locked: false },
      ],
    },
  }
  const same: RoutingConfiguration = {
    mesh: [{ src: ["tag:b", "tag:a"], dst: ["*"], via: [] }],
    egress: {
      rules: [
        {
          match: { domainSuffix: ["a.example", "b.example"] },
          action: { type: "exit", nodes: [`node:${A}`, `node:${B}`] },
        },
      ],
      assignments: [{ target: ["*"], allowed: ["mesh"], default: "mesh" }],
    },
  }
  assert.equal(sameRouting(base, same), true)
  assert.equal(sameRouting(base, writtenRouting(base)), true)
  const swapped = structuredClone(same)
  const rule = swapped.egress?.rules?.[0]
  if (rule?.action.type === "exit") rule.action.nodes.reverse()
  assert.equal(sameRouting(base, swapped), false)
})

test("written routing leaves defaults out", () => {
  const out = writtenRouting({
    mesh: [{ remark: "", src: ["*"], dst: ["*"], via: [] }],
    egress: {
      assignments: [
        { target: ["*"], allowed: ["mesh"], default: "mesh", locked: false },
      ],
      profiles: {
        work: { fallback: { type: "exit", nodes: [`node:${A}`], via: [] } },
      },
    },
  })
  assert.deepEqual(out, {
    mesh: [{ src: ["*"], dst: ["*"], via: [] }],
    egress: {
      rules: [],
      profiles: {
        work: { rules: [], fallback: { type: "exit", nodes: [`node:${A}`] } },
      },
      assignments: [{ target: ["*"], allowed: ["mesh"], default: "mesh" }],
    },
  })
})

test("renaming a profile or a rule set follows its references", () => {
  const routing: RoutingConfiguration = {
    egress: {
      rules: [{ match: { ruleSets: ["office"] }, action: { type: "direct" } }],
      profiles: {
        work: {
          rules: [
            {
              match: { ruleSets: ["office", "x"] },
              action: { type: "direct" },
            },
          ],
          fallback: { type: "unmanaged" },
        },
      },
      assignments: [
        { target: ["*"], allowed: ["work", "mesh"], default: "work" },
      ],
      auto: {
        rules: [{ countries: ["US"], profile: "work" }],
        fallback: "work",
      },
    },
  }
  const profile = renameProfile(routing, "work", "office-hours")
  assert.deepEqual(Object.keys(profile.egress?.profiles ?? {}), [
    "office-hours",
  ])
  assert.deepEqual(profile.egress?.assignments?.[0]?.allowed, [
    "office-hours",
    "mesh",
  ])
  assert.equal(profile.egress?.assignments?.[0]?.default, "office-hours")
  assert.equal(profile.egress?.auto?.fallback, "office-hours")
  assert.equal(profile.egress?.auto?.rules[0]?.profile, "office-hours")

  const sets = renameSetReferences(routing, "office", "hq")
  assert.deepEqual(referencedSets(sets).sort(), ["hq", "x"])
})

test("the profiles one can name: built-ins first, a global one per exit", () => {
  const routing: RoutingConfiguration = {
    egress: {
      profiles: { work: { fallback: { type: "unmanaged" } } },
      auto: { rules: [], fallback: "mesh" },
    },
  }
  assert.deepEqual(
    profileChoices(routing, [A], true).map((choice) => choice.value),
    ["mesh", "auto", `global:${A}`, "work"]
  )
  assert.deepEqual(
    profileChoices(routing, [], false).map((choice) => choice.value),
    ["mesh", "work"]
  )
})

test("problems sit at the controller's pointers", () => {
  const problems = routingProblems({
    mesh: [{ src: [], dst: ["bad selector"], via: ["tag:relay"] }],
    egress: {
      rules: [
        { match: {}, action: { type: "direct" } },
        {
          match: { ipCIDR: ["10.0.0.1/8"] },
          action: { type: "exit", nodes: [] },
        },
      ],
      profiles: { mesh: { fallback: { type: "exit", nodes: ["tag:x"] } } },
      assignments: [{ target: [], allowed: ["mesh"], default: "mesh" }],
      auto: {
        rules: [{ countries: ["us"], profile: "mesh" }],
        fallback: "mesh",
      },
    },
  })
  assert.deepEqual(problems, [
    { path: "/routing/mesh/0/src", code: "empty" },
    { path: "/routing/mesh/0/dst", code: "selector" },
    { path: "/routing/mesh/0/via", code: "node" },
    { path: "/routing/egress/rules/0/match", code: "match" },
    { path: "/routing/egress/rules/1/match", code: "destination" },
    { path: "/routing/egress/rules/1/action/nodes", code: "exits" },
    { path: "/routing/egress/profiles/mesh", code: "profileName" },
    { path: "/routing/egress/profiles/mesh/fallback/nodes", code: "node" },
    { path: "/routing/egress/assignments/0/target", code: "empty" },
    { path: "/routing/egress/auto/rules", code: "country" },
  ])
})
