import assert from "node:assert/strict"
import { test } from "node:test"

import type { MeshAccessPolicy, RoutingPolicy } from "@meshway/control-client"

// Tests use the generated runtime schemas directly; the product code receives
// them through the control client once it exports them.
import { zMeshAccessPolicy, zRoutingPolicy } from "../../../packages/control-client/src/generated/zod.gen.ts"
import {
  applyVisual,
  discardInvalidText,
  documentChange,
  editText,
  hasUnsavedWork,
  isValid,
  openDocument,
  parseFieldPath,
  restoreDocument,
  semanticProblems,
  settleSaved,
  storeDocument,
  type RuntimeSchema,
} from "./policy-document.ts"

const access: RuntimeSchema<MeshAccessPolicy> = zMeshAccessPolicy
const routing: RuntimeSchema<RoutingPolicy> = zRoutingPolicy

const base = { revision: 4, value: { grants: [{ id: "a", path: ["nyc", "sha"] }] } }

test("opens valid and unchanged", () => {
  const document = openDocument(base, access)
  assert.ok(isValid(document))
  assert.equal(hasUnsavedWork(document), false)
})

test("a comment-only edit is unsaved text but not a value change", () => {
  const document = editText(openDocument(base, access), `// note\n${openDocument(base, access).text}`, access)
  assert.ok(isValid(document))
  assert.deepEqual(documentChange(document), { valueChanged: false, textOnly: true })
  assert.ok(hasUnsavedWork(document))
})

test("syntax errors keep the last valid value and the typed text", () => {
  const opened = openDocument(base, access)
  const broken = editText(opened, `{"grants": [`, access)
  assert.equal(isValid(broken), false)
  assert.equal(broken.text, `{"grants": [`)
  assert.deepEqual(broken.value, base.value)
  assert.equal(broken.problems[0]?.source, "syntax")
  assert.equal(broken.problems[0]?.line, 1)
  assert.equal(broken.seq, 1)
})

test("schema issues point at the offending node and unknown keys", () => {
  const text = `{\n  "grants": [\n    { "id": "a", "path": ["nyc"], "extra": 1 }\n  ]\n}`
  const document = editText(openDocument(base, access), text, access)
  assert.equal(isValid(document), false)
  const unknown = document.problems.find((problem) => problem.path.at(-1) === "extra")
  assert.ok(unknown)
  assert.equal(text.slice(unknown.from, unknown.to), `"extra"`)
  assert.equal(unknown.line, 3)
  const short = document.problems.find((problem) => problem.path.join(".") === "grants.0.path")
  assert.ok(short)
  assert.equal(text.slice(short.from, short.to), `["nyc"]`)
})

test("routing enum errors are located", () => {
  const document = openDocument({ revision: 1, value: { default_action: "reject", rules: [] } }, routing)
  const edited = editText(document, `{ "default_action": "allow", "rules": [] }`, routing)
  const problem = edited.problems[0]
  assert.ok(problem)
  assert.equal(problem.source, "schema")
  assert.equal(edited.text.slice(problem.from, problem.to), `"allow"`)
})

test("visual edits are refused on an invalid buffer and never drop it", () => {
  const broken = editText(openDocument(base, access), `{"grants": [`, access)
  const result = applyVisual(broken, { grants: [] }, access)
  assert.equal(result.ok, false)
  assert.equal(broken.text, `{"grants": [`)
})

test("visual edits keep comments and bump seq", () => {
  const commented = `{\n  // keep\n  "grants": [\n    { "id": "a", "path": ["nyc", "sha"] } // first\n  ]\n}\n`
  const document = editText(openDocument(base, access), commented, access)
  const result = applyVisual(document, { grants: [{ id: "a", path: ["nyc", "sha"] }, { id: "b", path: ["sha", "nyc"] }] }, access)
  assert.ok(result.ok)
  assert.ok(result.document.text.includes("// keep"))
  assert.ok(result.document.text.includes("// first"))
  assert.equal(result.document.seq, document.seq + 1)
  assert.equal(result.document.value.grants.length, 2)
})

test("explicit discard returns to the last valid text, comments included", () => {
  const commented = `// keep\n{"grants": []}`
  const valid = editText(openDocument(base, access), commented, access)
  const broken = editText(valid, `// keep\n{"grants": [}`, access)
  const discarded = discardInvalidText(broken, access)
  assert.equal(discarded.text, commented)
  assert.ok(isValid(discarded))
})

test("a save settles only the work that was sent", () => {
  const edited = editText(openDocument(base, access), `// mine\n{"grants": []}`, access)
  const saved = settleSaved(edited, { revision: 5, value: { grants: [] } }, edited.seq, access)
  assert.equal(saved.base.revision, 5)
  assert.equal(saved.text, edited.text)
  assert.equal(hasUnsavedWork(saved), true, "comments are browser-only and still shown as unsaved text")
  assert.equal(documentChange(saved).valueChanged, false)

  const later = editText(edited, `{"grants": [{"id": "z", "path": ["a", "b"]}]}`, access)
  const settled = settleSaved(later, { revision: 5, value: { grants: [] } }, edited.seq, access)
  assert.equal(settled.text, later.text)
  assert.equal(documentChange(settled).valueChanged, true)
})

test("restoring keeps the invalid buffer and the last valid text", () => {
  const valid = editText(openDocument(base, access), `// v\n{"grants": []}`, access)
  const broken = editText(valid, `{"grants": [`, access)
  const restored = restoreDocument(storeDocument(broken), access)
  assert.equal(restored.text, broken.text)
  assert.equal(restored.validText, valid.text)
  assert.deepEqual(restored.value, { grants: [] })
  assert.equal(restored.seq, broken.seq)
})

test("semantic issues from field paths are placed on the buffer", () => {
  assert.deepEqual(parseFieldPath("grants[2].path[0]"), ["grants", 2, "path", 0])
  const document = openDocument(base, access)
  const [problem] = semanticProblems(document, [{ field: "grants[0].path[1]", message: "revoked" }])
  assert.ok(problem)
  assert.equal(document.text.slice(problem.from, problem.to), `"sha"`)
})
