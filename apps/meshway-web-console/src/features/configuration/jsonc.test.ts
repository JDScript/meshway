import assert from "node:assert/strict"
import { test } from "node:test"

import {
  commentRanges,
  findKey,
  formatJsonc,
  lineColumn,
  parseJsonc,
  pointerOf,
  pointerSegments,
  spanOfPointer,
} from "./jsonc.ts"

const TEXT = `{
  // who may reach whom
  "access": [{ "src": ["*"], "dst": ["*"], "ip": ["*"] }],
  "groups": { "group:a/b": ["x@example.test"] },
}
`

const treeOf = (text: string) => {
  const parsed = parseJsonc(text)
  if (!parsed.ok) throw new Error("expected valid JSONC")
  return parsed.tree
}

test("comments and trailing commas parse; duplicate keys do not", () => {
  const parsed = parseJsonc(TEXT)
  assert.ok(parsed.ok)
  assert.deepEqual(Object.keys(parsed.value as object), ["access", "groups"])

  const duplicate = parseJsonc('{ "a": 1, "a": 2 }')
  assert.equal(duplicate.ok, false)
  if (!duplicate.ok) assert.equal(duplicate.errors[0]?.code, "DuplicateKey")

  assert.equal(parseJsonc("   ").ok, false)
})

test("__proto__ stays data", () => {
  const parsed = parseJsonc('{ "__proto__": { "polluted": true } }')
  assert.ok(parsed.ok)
  assert.equal(({} as Record<string, unknown>).polluted, undefined)
  assert.ok(Object.hasOwn(parsed.value as object, "__proto__"))
})

test("pointers split and escape as RFC 6901 says", () => {
  assert.deepEqual(pointerSegments(""), [])
  assert.deepEqual(pointerSegments("/groups/group:a~1b/0"), [
    "groups",
    "group:a/b",
    "0",
  ])
  assert.deepEqual(pointerSegments("/a~01"), ["a~1"])
  assert.equal(pointerSegments("groups"), null)
  assert.equal(pointerOf(["groups", "group:a/b", 0]), "/groups/group:a~1b/0")
  assert.equal(pointerOf(["a~1"]), "/a~01")
})

test("a pointer names its value in the text", () => {
  const tree = treeOf(TEXT)
  const src = spanOfPointer(tree, "/access/0/src")
  assert.ok(src?.exact)
  assert.equal(TEXT.slice(src.from, src.to), '["*"]')

  const member = spanOfPointer(tree, "/groups/group:a~1b/0")
  assert.ok(member?.exact)
  assert.equal(TEXT.slice(member.from, member.to), '"x@example.test"')

  // Not in the text: the opening of the deepest value that holds the path.
  const missing = spanOfPointer(tree, "/access/0/bidirectional")
  assert.ok(missing && !missing.exact)
  assert.equal(TEXT.slice(missing.from, missing.to), "{")

  // An array index must be a canonical number.
  const loose = spanOfPointer(tree, "/access/00")
  assert.ok(loose && !loose.exact)

  assert.equal(spanOfPointer(tree, "access"), null)
  const whole = spanOfPointer(tree, "")
  assert.deepEqual([whole?.from, whole?.exact], [0, true])
})

test("a key is found where it is written", () => {
  const tree = treeOf(TEXT)
  const key = findKey(tree, ["groups"], "group:a/b")
  assert.ok(key)
  assert.equal(TEXT.slice(key.offset, key.offset + key.length), '"group:a/b"')
  assert.equal(findKey(tree, ["groups"], "group:none"), undefined)
})

test("lines and columns count from one", () => {
  const at = TEXT.indexOf('"groups"')
  assert.deepEqual(lineColumn(TEXT, at), { line: 4, column: 3 })
  assert.deepEqual(lineColumn("a\r\nb", 3), { line: 2, column: 1 })
})

test("comments are found for the editor", () => {
  const ranges = commentRanges(TEXT)
  assert.equal(ranges.length, 1)
  const [first] = ranges
  assert.ok(first)
  assert.equal(TEXT.slice(first.from, first.to), "// who may reach whom")
})

test("formatting keeps comments and values", () => {
  const text = '{"a":[1,2], // keep\n"b":{"c":true}}'
  const formatted = formatJsonc(text)
  assert.ok(formatted.includes("// keep"))
  assert.ok(formatted.endsWith("\n"))
  const before = parseJsonc(text)
  const after = parseJsonc(formatted)
  assert.ok(before.ok && after.ok)
  assert.deepEqual(after.value, before.value)
  assert.equal(formatJsonc(formatted), formatted)
})
