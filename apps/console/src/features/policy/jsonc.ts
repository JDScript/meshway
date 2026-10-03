import {
  applyEdits,
  createScanner,
  findNodeAtLocation,
  modify,
  parseTree,
  printParseErrorCode,
  type Node,
  type ParseError,
} from "jsonc-parser"

import { sameData } from "../../lib/same-data.ts"

// JSONC (JSON with comments and trailing commas) as the controller reads it,
// on top of `jsonc-parser`: duplicate keys are errors (the controller rejects
// them; last-wins would hide a typed value), values are built without the
// `__proto__` setter, and edits made in the graphical views are applied to
// the text as the smallest set of changes, so comments and formatting
// outside the touched values stay.

export type JsoncNode = Node
export type JsonPath = readonly (string | number)[]

export type JsoncErrorCode =
  ReturnType<typeof printParseErrorCode> | "DuplicateKey" | "Empty"

type JsoncError = {
  code: JsoncErrorCode
  offset: number
  length: number
}

type JsoncParse =
  | { ok: true; tree: JsoncNode; value: unknown }
  | { ok: false; errors: JsoncError[] }

const OPTIONS = {
  disallowComments: false,
  allowTrailingComma: true,
  allowEmptyContent: false,
}

export function parseJsonc(text: string): JsoncParse {
  const raw: ParseError[] = []
  const tree = parseTree(text, raw, OPTIONS)
  const errors: JsoncError[] = raw.map((error) => ({
    code: printParseErrorCode(error.error),
    offset: error.offset,
    length: error.length,
  }))
  if (!tree && errors.length === 0)
    errors.push({ code: "Empty", offset: 0, length: 0 })
  if (tree) duplicateKeys(tree, errors)
  if (!tree || errors.length > 0) {
    errors.sort((a, b) => a.offset - b.offset)
    return { ok: false, errors }
  }
  return { ok: true, tree, value: nodeValue(tree) }
}

function duplicateKeys(node: JsoncNode, errors: JsoncError[]) {
  if (node.type === "object") {
    const seen = new Set<string>()
    for (const property of node.children ?? []) {
      const key = property.children?.[0]
      const name = propertyKey(property)
      if (key && name !== undefined) {
        if (seen.has(name))
          errors.push({
            code: "DuplicateKey",
            offset: key.offset,
            length: key.length,
          })
        seen.add(name)
      }
    }
  }
  for (const child of node.children ?? []) duplicateKeys(child, errors)
}

/** The plain value of a node. Keys are defined, never assigned, so `__proto__` stays data. */
function nodeValue(node: JsoncNode): unknown {
  switch (node.type) {
    case "object": {
      const result: Record<string, unknown> = {}
      for (const property of node.children ?? []) {
        const key = propertyKey(property)
        const value = property.children?.[1]
        if (key === undefined || !value) continue
        Object.defineProperty(result, key, {
          value: nodeValue(value),
          enumerable: true,
          writable: true,
          configurable: true,
        })
      }
      return result
    }
    case "array":
      return (node.children ?? []).map(nodeValue)
    case "property":
      return node.children?.[1] ? nodeValue(node.children[1]) : undefined
    default: {
      const value: unknown = node.value
      return value
    }
  }
}

function propertyKey(property: JsoncNode): string | undefined {
  const value: unknown = property.children?.[0]?.value
  return typeof value === "string" ? value : undefined
}

/** The deepest node that exists along `path`; `exact` when it is the whole path. */
export function findNearest(
  root: JsoncNode,
  path: JsonPath
): { node: JsoncNode; exact: boolean } {
  let node = root
  for (const segment of path) {
    const next = findNodeAtLocation(node, [segment])
    if (!next) return { node, exact: false }
    node = next
  }
  return { node, exact: true }
}

/** The key of `key` inside the object at `path` (for an unknown field). */
export function findKey(
  root: JsoncNode,
  path: JsonPath,
  key: string
): JsoncNode | undefined {
  const { node, exact } = findNearest(root, path)
  if (!exact || node.type !== "object") return undefined
  return node.children?.find((property) => propertyKey(property) === key)
    ?.children?.[0]
}

/** 1-based line and column, in UTF-16 code units as editors count. */
export function lineColumn(
  text: string,
  offset: number
): { line: number; column: number } {
  let line = 1
  let lineStart = 0
  const end = Math.min(Math.max(offset, 0), text.length)
  for (let index = 0; index < end; index += 1) {
    const char = text[index]
    if (char === "\n" || (char === "\r" && text[index + 1] !== "\n")) {
      line += 1
      lineStart = index + 1
    }
  }
  return { line, column: end - lineStart + 1 }
}

// `SyntaxKind` is a const enum in the package typings, which isolated
// modules cannot read; these are its stable values.
const COMMA = 5
const LINE_COMMENT = 12
const BLOCK_COMMENT = 13
const EOF = 17

/** Every comment's range (the JSON grammar has none, so the editor marks them from this). */
export function commentRanges(text: string): { from: number; to: number }[] {
  const scanner = createScanner(text, false)
  const ranges: { from: number; to: number }[] = []
  for (let token = scanner.scan(); token !== EOF; token = scanner.scan()) {
    if (token === LINE_COMMENT || token === BLOCK_COMMENT) {
      const from = scanner.getTokenOffset()
      ranges.push({ from, to: from + scanner.getTokenLength() })
    }
  }
  return ranges
}

// --- Edits -----------------------------------------------------------------

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)

const own = (value: Record<string, unknown>, key: string) =>
  Object.prototype.hasOwnProperty.call(value, key)

type Operation = { path: (string | number)[]; value: unknown; insert: boolean }

/**
 * The operations turning `before` into `after`, deepest first. In an array,
 * paired elements are diffed in place, surplus elements are removed from the
 * highest index down and new ones inserted in ascending order, so every
 * path is still valid when its operation is applied.
 */
function operations(
  path: (string | number)[],
  before: unknown,
  after: unknown,
  out: Operation[]
) {
  if (sameData(before, after)) return
  if (isRecord(before) && isRecord(after)) {
    for (const key of Object.keys(before)) {
      if (own(after, key) && after[key] !== undefined)
        operations([...path, key], before[key], after[key], out)
      else out.push({ path: [...path, key], value: undefined, insert: false })
    }
    for (const key of Object.keys(after)) {
      if (!own(before, key) && after[key] !== undefined)
        out.push({ path: [...path, key], value: after[key], insert: false })
    }
    return
  }
  if (Array.isArray(before) && Array.isArray(after)) {
    let prefix = 0
    while (
      prefix < before.length &&
      prefix < after.length &&
      sameData(before[prefix], after[prefix])
    )
      prefix += 1
    let suffix = 0
    while (
      suffix < before.length - prefix &&
      suffix < after.length - prefix &&
      sameData(
        before[before.length - 1 - suffix],
        after[after.length - 1 - suffix]
      )
    )
      suffix += 1
    const oldMiddle = before.length - prefix - suffix
    const newMiddle = after.length - prefix - suffix
    const paired = Math.min(oldMiddle, newMiddle)
    for (let index = prefix; index < prefix + paired; index += 1)
      operations([...path, index], before[index], after[index], out)
    for (
      let index = prefix + oldMiddle - 1;
      index >= prefix + paired;
      index -= 1
    )
      out.push({ path: [...path, index], value: undefined, insert: false })
    for (let index = prefix + paired; index < prefix + newMiddle; index += 1)
      out.push({ path: [...path, index], value: after[index], insert: true })
    return
  }
  out.push({ path, value: after, insert: false })
}

/** The offset of the next token at or after `from`, when it is a comma. */
function commaFrom(text: string, from: number): number | null {
  const scanner = createScanner(text, true)
  scanner.setPosition(from)
  scanner.scan()
  return scanner.getToken() === COMMA ? scanner.getTokenOffset() : null
}

function treeOf(text: string): JsoncNode {
  const parsed = parseJsonc(text)
  if (!parsed.ok) throw new Error("cannot edit invalid JSONC")
  return parsed.tree
}

/**
 * Removes the element or property at `path` with one separating comma.
 * Unlike `modify` it never deletes comments: only the value, its comma and a
 * line the removal left blank go.
 */
function removeAt(text: string, path: JsonPath): string {
  const tree = treeOf(text)
  const last = path[path.length - 1]
  const parent = findNodeAtLocation(tree, [...path.slice(0, -1)])
  if (!parent || last === undefined) throw new Error("JSONC path not found")
  const children = parent.children ?? []
  const index =
    typeof last === "number"
      ? last
      : children.findIndex((property) => propertyKey(property) === last)
  const child = children[index]
  if (!child) throw new Error("JSONC path not found")
  const end = child.offset + child.length
  const ranges: { from: number; to: number }[] = [
    { from: child.offset, to: end },
  ]
  const after = commaFrom(text, end)
  const previous = children[index - 1]
  if (after !== null) {
    let to = after + 1
    while (text[to] === " " || text[to] === "\t") to += 1
    if (/^\s*$/.test(text.slice(end, after)))
      ranges[0] = { from: child.offset, to }
    else ranges.push({ from: after, to })
  } else if (previous) {
    const before = commaFrom(text, previous.offset + previous.length)
    if (before !== null && /^\s*$/.test(text.slice(before + 1, child.offset)))
      ranges[0] = { from: before, to: end }
    else if (before !== null) ranges.push({ from: before, to: before + 1 })
  }
  ranges.sort((a, b) => b.from - a.from)
  let result = text
  for (const range of ranges) {
    result = result.slice(0, range.from) + result.slice(range.to)
    let start = range.from
    while (
      start > 0 &&
      (result[start - 1] === " " || result[start - 1] === "\t")
    )
      start -= 1
    let stop = range.from
    while (
      stop < result.length &&
      (result[stop] === " " || result[stop] === "\t")
    )
      stop += 1
    const lineStart = start === 0 || result[start - 1] === "\n"
    if (lineStart && result[stop] === "\n")
      result = result.slice(0, start) + result.slice(stop + 1)
    else if (lineStart && result[stop] === "\r" && result[stop + 1] === "\n")
      result = result.slice(0, start) + result.slice(stop + 2)
  }
  return result
}

const WIDTH = 100

/** The text's indentation unit (its first indented line), else two spaces. */
function unitOf(text: string): string {
  return /\n([ \t]+)\S/.exec(text)?.[1] ?? "  "
}

/** The whitespace that starts the line holding `offset`. */
function indentAt(text: string, offset: number): string {
  let start = offset
  while (start > 0 && text[start - 1] !== "\n") start -= 1
  return /^[ \t]*/.exec(text.slice(start))?.[0] ?? ""
}

/** One line: `{ "a": 1, "b": ["x"] }`. */
function compact(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(compact).join(", ")}]`
  if (isRecord(value)) {
    const entries = Object.entries(value).filter(
      ([, item]) => item !== undefined
    )
    return entries.length === 0
      ? "{}"
      : `{ ${entries.map(([key, item]) => `${JSON.stringify(key)}: ${compact(item)}`).join(", ")} }`
  }
  return JSON.stringify(value)
}

/** A value as it would be written at `indent`: on one line when it fits, else one entry per line. */
function printValue(
  value: unknown,
  indent: string,
  unit: string,
  eol: string,
  lead = 0
): string {
  const line = compact(value)
  if (indent.length + lead + line.length <= WIDTH) return line
  const inner = indent + unit
  if (Array.isArray(value) && value.length > 0)
    return `[${eol}${value.map((item) => inner + printValue(item, inner, unit, eol)).join(`,${eol}`)}${eol}${indent}]`
  if (isRecord(value)) {
    const entries = Object.entries(value).filter(
      ([, item]) => item !== undefined
    )
    if (entries.length > 0)
      return `{${eol}${entries
        .map(([key, item]) => {
          const name = `${JSON.stringify(key)}: `
          return inner + name + printValue(item, inner, unit, eol, name.length)
        })
        .join(`,${eol}`)}${eol}${indent}}`
  }
  return line
}

/** The end of the line `from` is on, when only spaces or a line comment follow it there; else `from`. */
function lineEnd(text: string, from: number): number {
  const rest = /^[ \t]*(\/\/[^\n\r]*)?(?=\r?\n|$)/.exec(text.slice(from))
  return rest ? from + rest[0].length : from
}

/** Inserts `value` as element `index` of the array at `path`, in the array's own layout. */
function insertAt(
  text: string,
  path: JsonPath,
  index: number,
  value: unknown
): string {
  const array = findNodeAtLocation(treeOf(text), [...path])
  if (!array || array.type !== "array") throw new Error("JSONC array not found")
  const elements = array.children ?? []
  const unit = unitOf(text)
  const eol = text.includes("\r\n") ? "\r\n" : "\n"
  const own = text.slice(array.offset, array.offset + array.length)
  const lineIndent = indentAt(text, array.offset)
  const multiline =
    elements.length > 0
      ? /\n/.test(own)
      : isRecord(value) || Array.isArray(value)
  const splice = (at: number, insert: string, to = at) =>
    text.slice(0, at) + insert + text.slice(to)

  if (elements.length === 0) {
    const open = array.offset + 1
    const close = array.offset + array.length - 1
    // Keep whatever sits inside the empty brackets (a comment).
    const inside = text.slice(open, close)
    if (!multiline) return splice(open, compact(value) + inside.trim(), close)
    const inner = lineIndent + unit
    return splice(
      open,
      `${eol}${inner}${printValue(value, inner, unit, eol)}${inside.trimEnd() === "" ? "" : inside.trimEnd()}${eol}${lineIndent}`,
      close
    )
  }

  const reference = elements[Math.min(index, elements.length - 1)]
  if (!reference) throw new Error("JSONC element not found")
  const indent = multiline ? indentAt(text, reference.offset) : ""
  const printed = multiline
    ? printValue(value, indent, unit, eol)
    : compact(value)

  if (index === 0) {
    const open = array.offset + 1
    return multiline
      ? splice(open, `${eol}${indent}${printed},`)
      : splice(reference.offset, `${printed}, `)
  }
  const previous = elements[index - 1]
  if (!previous) throw new Error("JSONC element not found")
  const end = previous.offset + previous.length
  const comma = commaFrom(text, end)
  if (!multiline) return splice(end, `, ${printed}`)
  if (comma === null) {
    // The previous element was last with no trailing comma: add one, then the new element.
    const at = lineEnd(text, end)
    return (
      text.slice(0, end) +
      "," +
      text.slice(end, at) +
      `${eol}${indent}${printed}` +
      text.slice(at)
    )
  }
  // After the comma and anything else on its line (a comment): a separator
  // gets one after the new element, and a trailing comma stays trailing.
  return splice(lineEnd(text, comma + 1), `${eol}${indent}${printed},`)
}

/** Replaces the value at `path` (which exists) in place. */
function replaceAt(text: string, path: JsonPath, value: unknown): string {
  const node = findNodeAtLocation(treeOf(text), [...path])
  if (!node) throw new Error("JSONC path not found")
  const unit = unitOf(text)
  const eol = text.includes("\r\n") ? "\r\n" : "\n"
  const lead = node.offset - (text.lastIndexOf("\n", node.offset - 1) + 1)
  const indent = indentAt(text, node.offset)
  const printed = printValue(value, indent, unit, eol, lead - indent.length)
  return (
    text.slice(0, node.offset) + printed + text.slice(node.offset + node.length)
  )
}

/**
 * Rewrites `text` so that it parses to `after`, touching only the deepest
 * values that differ and writing new values in the layout around them.
 * Throws when `text` is not valid JSONC: a graphical edit is never applied
 * over text it was not read from.
 */
export function patchJsonc(text: string, after: unknown): string {
  const parsed = parseJsonc(text)
  if (!parsed.ok) throw new Error("cannot edit invalid JSONC")
  const out: Operation[] = []
  operations([], parsed.value, after, out)
  let result = text
  for (const operation of out) {
    if (operation.path.length === 0) return formatJsonc(after)
    const last = operation.path[operation.path.length - 1]
    if (operation.value === undefined) result = removeAt(result, operation.path)
    else if (operation.insert && typeof last === "number")
      result = insertAt(
        result,
        operation.path.slice(0, -1),
        last,
        operation.value
      )
    else if (findNodeAtLocation(treeOf(result), operation.path))
      result = replaceAt(result, operation.path, operation.value)
    else
      result = applyEdits(
        result,
        modify(result, operation.path, operation.value, {})
      )
  }
  const check = parseJsonc(result)
  if (!check.ok || !sameData(check.value, after))
    throw new Error("JSONC edit did not produce the edited value")
  return result
}

/**
 * Swaps the element at `index` of the array at `path` with its neighbour
 * (`delta` −1 or +1). Each element moves with the comments and lines above
 * it, so a note written over a rule stays with that rule.
 */
export function moveJsonc(
  text: string,
  path: JsonPath,
  index: number,
  delta: -1 | 1
): string {
  const tree = treeOf(text)
  const array = findNodeAtLocation(tree, [...path])
  if (!array || array.type !== "array") throw new Error("JSONC array not found")
  const elements = array.children ?? []
  const first = Math.min(index, index + delta)
  const a = elements[first]
  const b = elements[first + 1]
  if (!a || b === undefined) throw new Error("JSONC element not found")
  const separator = commaFrom(text, a.offset + a.length)
  if (separator === null) throw new Error("JSONC elements not separated")
  // Each block runs from just after the previous separator (and the spaces
  // on its line) to the end of its element.
  const skipSpaces = (from: number) => {
    let at = from
    while (text[at] === " " || text[at] === "\t") at += 1
    return at
  }
  const previous = elements[first - 1]
  const aFrom = skipSpaces(
    previous === undefined
      ? array.offset + 1
      : (commaFrom(text, previous.offset + previous.length) ?? a.offset - 1) + 1
  )
  const aTo = a.offset + a.length
  const bFrom = skipSpaces(separator + 1)
  const bTo = b.offset + b.length
  const blockA = text.slice(aFrom, aTo)
  const blockB = text.slice(bFrom, bTo)
  const result =
    text.slice(0, aFrom) +
    blockB +
    text.slice(aTo, bFrom) +
    blockA +
    text.slice(bTo)
  const before = nodeValue(array)
  const check = parseJsonc(result)
  if (!check.ok || !Array.isArray(before)) throw new Error("JSONC move failed")
  const expected = [...before]
  ;[expected[first], expected[first + 1]] = [
    expected[first + 1],
    expected[first],
  ]
  const moved = findNodeAtLocation(check.tree, [...path])
  if (!moved || !sameData(nodeValue(moved), expected))
    throw new Error("JSONC move did not produce the moved value")
  return result
}

/**
 * Text for a document with no text before it: one section per line, one
 * element of each list per line (on one line when it fits), keys in `order`
 * first and the rest after them.
 */
export function formatJsonc(
  value: unknown,
  order: readonly string[] = []
): string {
  const rank = (key: string) => {
    const at = order.indexOf(key)
    return at < 0 ? order.length : at
  }
  const sorted = (item: unknown): unknown => {
    if (Array.isArray(item)) return item.map(sorted)
    if (!isRecord(item)) return item
    const out: Record<string, unknown> = {}
    for (const key of Object.keys(item).sort((a, b) => rank(a) - rank(b)))
      Object.defineProperty(out, key, {
        value: sorted(item[key]),
        enumerable: true,
        writable: true,
        configurable: true,
      })
    return out
  }
  const root = sorted(value)
  if (!isRecord(root)) return `${printValue(root, "", "  ", "\n")}\n`
  const entries = Object.entries(root).filter(([, item]) => item !== undefined)
  const lines = entries.map(([key, item]) => {
    const name = `  ${JSON.stringify(key)}: `
    if (Array.isArray(item) && item.length > 0)
      return `${name}[\n${item.map((element) => `    ${printValue(element, "    ", "  ", "\n")}`).join(",\n")}\n  ]`
    return name + printValue(item, "  ", "  ", "\n", name.length - 2)
  })
  return `{\n${lines.join(",\n")}\n}\n`
}
