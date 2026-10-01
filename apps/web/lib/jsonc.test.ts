import assert from "node:assert/strict"
import { test } from "node:test"

import { findNode, formatJsonc, lineColumn, parseJsonc, patchJsonc } from "./jsonc.ts"

test("parses comments and trailing commas", () => {
  const text = `{
  // routing
  "rules": [1, 2,], /* block */
  "default_action": "reject",
}`
  const parsed = parseJsonc(text)
  assert.deepEqual(parsed.errors, [])
  assert.deepEqual(parsed.value, { rules: [1, 2], default_action: "reject" })
  assert.ok(parsed.tree)
  const node = findNode(parsed.tree, ["rules", 1])
  assert.ok(node)
  assert.equal(text.slice(node.offset, node.offset + node.length), "2")
})

test("reports the first syntax error with a position", () => {
  const text = `{\n  "a": 1\n  "b": 2\n}`
  const parsed = parseJsonc(text)
  assert.equal(parsed.tree, null)
  assert.equal(parsed.value, undefined)
  const [error] = parsed.errors
  assert.ok(error)
  assert.equal(error.code, "CommaExpected")
  assert.deepEqual(lineColumn(text, error.offset), { line: 3, column: 3 })
})

test("rejects duplicate keys instead of silently keeping the last", () => {
  assert.equal(parseJsonc(`{"a": 1, "a": 2}`).errors[0]?.code, "DuplicateKey")
})

test("keeps __proto__ as data", () => {
  const value = parseJsonc(`{"__proto__": {"x": 1}}`).value
  assert.ok(typeof value === "object" && value !== null)
  assert.deepEqual(Object.keys(value), ["__proto__"])
  assert.equal(Object.getPrototypeOf(value), Object.prototype)
})

test("rejects unterminated comments, strings, and stray symbols", () => {
  assert.equal(parseJsonc(`{"a": 1 /* x`).errors[0]?.code, "UnexpectedEndOfComment")
  assert.equal(parseJsonc(`{"a": "x`).errors[0]?.code, "UnexpectedEndOfString")
  assert.ok(parseJsonc(`{"a": 01}`).errors.length > 0)
  assert.ok(parseJsonc(`{a: 1}`).errors.length > 0)
  assert.ok(parseJsonc(`{} x`).errors.length > 0)
})

const commented = `{
  // who may talk
  "grants": [
    // nyc to sha
    { "id": "a", "path": ["nyc", "sha"] },
    { "id": "b", "path": ["sha", "nyc"] }, // reverse
  ],
}
`

function roundTrip(after: unknown, text = commented) {
  const before = parseJsonc(text).value
  const patched = patchJsonc(text, before, after)
  assert.deepEqual(parseJsonc(patched).errors, [], patched)
  assert.deepEqual(parseJsonc(patched).value, after, patched)
  return patched
}

test("replaces only the changed scalar and keeps every comment", () => {
  const patched = roundTrip({
    grants: [
      { id: "a", path: ["nyc", "lax", "sha"] },
      { id: "b", path: ["sha", "nyc"] },
    ],
  })
  for (const comment of ["// who may talk", "// nyc to sha", "// reverse"]) assert.ok(patched.includes(comment))
})

test("appends to an array after a trailing comma", () => {
  const patched = roundTrip({
    grants: [
      { id: "a", path: ["nyc", "sha"] },
      { id: "b", path: ["sha", "nyc"] },
      { id: "c", path: ["nyc", "lax"] },
    ],
  })
  assert.ok(patched.includes("// reverse"))
})

test("removes the first, last, or all elements", () => {
  const first = roundTrip({ grants: [{ id: "b", path: ["sha", "nyc"] }] })
  assert.ok(first.includes("// who may talk"))
  roundTrip({ grants: [{ id: "a", path: ["nyc", "sha"] }] })
  roundTrip({ grants: [] })
})

test("inserts in the middle and removes consecutive runs", () => {
  const text = `[1, 2, 3, 4, 5]`
  roundTrip([1, 9, 2, 3, 4, 5], text)
  roundTrip([1, 5], text)
  roundTrip([1, 2], text)
  roundTrip([2, 1, 3, 4, 5], text)
})

test("adds and removes object properties", () => {
  const text = `{\n  "a": 1, // one\n  "b": 2,\n  "c": 3\n}`
  assert.ok(roundTrip({ a: 1, c: 3, d: 4 }, text).includes("// one"))
  roundTrip({ a: 1, b: 2, d: 4 }, text)
  roundTrip({ x: 1 }, text)
  roundTrip({ a: 1, b: 2, c: 3, d: { e: [1] } }, `{"a": 1, "b": 2, "c": 3}`)
  roundTrip({ a: 1 }, `{}`)
  roundTrip([1], `[]`)
})

test("removing never deletes comments", () => {
  const text = `[\n  1, // one\n  2, // two\n  3\n]`
  assert.equal(roundTrip([1, 3], text), `[\n  1, // one\n  // two\n  3\n]`)
  assert.equal(roundTrip([1, 2], text), `[\n  1, // one\n  2 // two\n]`)
  assert.equal(roundTrip([1, 3], `[1, 2, 3]`), `[1, 3]`)
  assert.equal(roundTrip([1, 2], `[1, 2, 3]`), `[1, 2]`)
  const object = `{\n  "a": 1, // one\n  "b": 2,\n  "c": 3\n}`
  assert.ok(roundTrip({ a: 1, c: 3 }, object).includes("// one"))
  assert.ok(roundTrip({ a: 1, b: 2 }, object).includes("// one"))
})

test("refuses to patch a buffer it was not derived from", () => {
  assert.throws(() => patchJsonc(`{"a": 1`, { a: 1 }, { a: 2 }))
  assert.throws(() => patchJsonc(`{"a": 1}`, { a: 2 }, { a: 3 }))
})

test("formats a value with a trailing newline", () => {
  assert.equal(formatJsonc({ a: [1] }), `{\n  "a": [\n    1\n  ]\n}\n`)
})
