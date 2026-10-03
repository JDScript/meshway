import assert from "node:assert/strict"
import { test } from "node:test"

import {
  editPolicy,
  fieldPath,
  locate,
  movePolicy,
  placeDiagnostic,
  policyText,
  readDocument,
  TEMPLATE_POLICY,
} from "./document.ts"
import { commentRanges, moveJsonc, parseJsonc, patchJsonc } from "./jsonc.ts"

const SOURCE = `{
  // Every node may dial every other.
  "connections": [
    { "id": "default", "a": "*", "b": "*", "action": "allow" }, // template
  ],
  "grants": [
    /* members reach members */
    { "id": "members", "src": ["*"], "dst": ["*"], "bidirectional": false },
  ],
  "mesh": [
    // office first
    { "id": "office", "src": "tag:office", "dst": "*", "via": ["tag:hub"] },
    // then the rest
    { "id": "rest", "src": "*", "dst": "*", "via": [] },
  ],
  "public_rules": [],
  "profiles": [],
  "assignments": [],
}
`

test("reads a commented policy with trailing commas", () => {
  const document = readDocument(SOURCE)
  assert.deepEqual(document.problems, [])
  assert.ok(document.policy)
  assert.equal(document.policy.mesh[1]?.id, "rest")
})

test("a graphical edit keeps every comment and the layout", () => {
  const next = editPolicy(SOURCE, (policy) => ({
    ...policy,
    connections: policy.connections.map((rule) => ({
      ...rule,
      action: "block",
    })),
  }))
  assert.notEqual(next, SOURCE)
  assert.deepEqual(
    commentRanges(next).map((range) => next.slice(range.from, range.to)),
    commentRanges(SOURCE).map((range) => SOURCE.slice(range.from, range.to))
  )
  assert.equal(next, SOURCE.replace('"action": "allow"', '"action": "block"'))
})

test("adding and removing rules leaves comments in place", () => {
  const added = editPolicy(SOURCE, (policy) => ({
    ...policy,
    grants: [
      ...policy.grants,
      { id: "ops", src: ["tag:ops"], dst: ["*"], bidirectional: true },
    ],
  }))
  assert.ok(added.includes("/* members reach members */"))
  assert.equal(readDocument(added).policy?.grants.length, 2)
  const removed = editPolicy(added, (policy) => ({
    ...policy,
    grants: policy.grants.filter((grant) => grant.id !== "members"),
  }))
  assert.ok(removed.includes("/* members reach members */"))
  assert.deepEqual(
    readDocument(removed).policy?.grants.map((grant) => grant.id),
    ["ops"]
  )
})

test("moving a rule carries the comment written above it", () => {
  const moved = movePolicy(SOURCE, ["mesh"], 1, -1)
  const policy = readDocument(moved).policy
  assert.deepEqual(
    policy?.mesh.map((rule) => rule.id),
    ["rest", "office"]
  )
  assert.ok(
    moved.indexOf("// then the rest") < moved.indexOf('"id": "rest"') &&
      moved.indexOf('"id": "rest"') < moved.indexOf("// office first") &&
      moved.indexOf("// office first") < moved.indexOf('"id": "office"')
  )
  assert.equal(movePolicy(moved, ["mesh"], 0, 1), SOURCE)
})

test("moves within one line keep the spacing", () => {
  assert.equal(moveJsonc('["a", "b", "c"]', [], 2, -1), '["a", "c", "b"]')
  assert.equal(moveJsonc('["a", "b"]', [], 0, 1), '["b", "a"]')
})

test("invalid text is never rewritten by a graphical edit", () => {
  const broken = SOURCE.replace('"rest", ', '"rest" ')
  assert.equal(readDocument(broken).policy, null)
  assert.equal(readDocument(broken).problems[0]?.kind, "syntax")
  assert.equal(
    editPolicy(broken, (policy) => ({ ...policy, mesh: [] })),
    broken
  )
  assert.equal(movePolicy(broken, ["mesh"], 0, 1), broken)
  // Valid JSONC that is not a policy: also left alone.
  const wrong = SOURCE.replace(
    '"bidirectional": false',
    '"bidirectional": "no"'
  )
  const document = readDocument(wrong)
  assert.equal(document.policy, null)
  assert.equal(document.problems[0]?.kind, "schema")
  assert.equal(
    editPolicy(wrong, (policy) => ({ ...policy, grants: [] })),
    wrong
  )
})

test("schema problems point at the offending text", () => {
  const text = SOURCE.replace('"action": "allow"', '"action": "permit"')
  const [problem] = readDocument(text).problems
  assert.ok(problem && problem.kind === "schema")
  assert.equal(text.slice(problem.from, problem.to), '"permit"')
  const unknown = SOURCE.replace('"via": []', '"via": [], "extra": 1')
  const [extra] = readDocument(unknown).problems
  assert.ok(extra && extra.kind === "schema" && extra.code === "unknown_key")
  assert.equal(unknown.slice(extra.from, extra.to), '"extra"')
})

test("duplicate keys and duplicate rule IDs are problems", () => {
  const keys = SOURCE.replace('"id": "rest",', '"id": "rest", "id": "x",')
  assert.equal(readDocument(keys).problems[0]?.kind, "syntax")
  const ids = SOURCE.replace('"id": "rest"', '"id": "members"')
  const document = readDocument(ids)
  const [problem] = document.problems
  assert.ok(problem && problem.kind === "id" && problem.code === "duplicate")
  assert.equal(ids.slice(problem.from, problem.to), '"members"')
  // The views stay while an ID is wrong, and edits still apply.
  assert.ok(document.policy)
  assert.notEqual(
    editPolicy(ids, (policy) => ({ ...policy, mesh: [] })),
    ids
  )
})

test("the template is a valid policy", () => {
  assert.deepEqual(
    readDocument(policyText(TEMPLATE_POLICY)).policy,
    TEMPLATE_POLICY
  )
})

test("diagnostics follow their rule by ID", () => {
  assert.deepEqual(fieldPath("grants[1].dst[0]"), ["grants", 1, "dst", 0])
  assert.deepEqual(fieldPath("automatic.regions[CN]"), [
    "automatic",
    "regions",
    "CN",
  ])
  const saved = readDocument(SOURCE).policy
  assert.ok(saved)
  const lint = placeDiagnostic(
    { field: "mesh.rest", message: "shadowed" },
    saved
  )
  assert.deepEqual(lint.placed, {
    kind: "field",
    anchor: ["mesh", { id: "rest" }],
  })
  const field = placeDiagnostic(
    { field: "mesh[0].via[0]", message: "Unknown" },
    saved
  )
  assert.deepEqual(field.placed, {
    kind: "field",
    anchor: ["mesh", { id: "office" }, "via", 0],
  })
  const failure = placeDiagnostic(
    { field: `mesh.${"a".repeat(64)}`, message: "x" },
    saved
  )
  assert.deepEqual(failure.placed, { kind: "path", nodeId: "a".repeat(64) })
  assert.deepEqual(
    placeDiagnostic({ field: "nodes", message: "x" }, saved).placed,
    {
      kind: "general",
    }
  )
  // After the draft reorders, the anchor still finds the rule.
  const moved = readDocument(movePolicy(SOURCE, ["mesh"], 1, -1)).policy
  assert.ok(field.placed.kind === "field")
  assert.deepEqual(locate(moved, field.placed.anchor), ["mesh", 1, "via", 0])
  assert.equal(locate({ ...moved, mesh: [] }, field.placed.anchor), null)
})

test("parse errors are reported in text order", () => {
  const parsed = parseJsonc('{ "a": , }')
  assert.equal(parsed.ok, false)
  assert.throws(() => patchJsonc("{", {}))
})

test("new values are written in the layout around them", () => {
  const text = `{
  "list": [
    { "id": "a" }, // first
  ],
  "inline": ["x"],
  "empty": [],
}
`
  assert.equal(
    patchJsonc(text, {
      list: [{ id: "a" }, { id: "b" }],
      inline: ["x", "y"],
      empty: [{ id: "c" }],
    }),
    `{
  "list": [
    { "id": "a" }, // first
    { "id": "b" },
  ],
  "inline": ["x", "y"],
  "empty": [
    { "id": "c" }
  ],
}
`
  )
  // A last element without a comma gets one before the new element.
  assert.equal(
    patchJsonc('{\n  "l": [\n    1\n  ]\n}', { l: [1, 2] }),
    '{\n  "l": [\n    1,\n    2\n  ]\n}'
  )
})
