import {
  applyEdits as applyJsoncEdits,
  createScanner,
  findNodeAtLocation,
  modify,
  parseTree,
  printParseErrorCode,
  type FormattingOptions,
  type Node,
  type ParseError,
} from "jsonc-parser"

// JSONC (JSON with comments and trailing commas) for the policy text editors,
// on top of `jsonc-parser`. On top of the package this module adds what the
// policy editors need: duplicate keys are errors (the controller rejects
// them, and last-wins would silently hide a typed value), values are built
// without the `__proto__` setter, and `patchJsonc` turns a visual edit into
// the smallest set of `modify` calls so comments outside touched nodes stay.

export type JsoncNode = Node
export type JsoncNodeType = Node["type"]

export type JsoncErrorCode = ReturnType<typeof printParseErrorCode> | "DuplicateKey"

export type JsoncError = { code: JsoncErrorCode; offset: number; length: number; message: string }

export type JsoncParse = {
  /** Null when there is any syntax error. */
  tree: JsoncNode | null
  /** The parsed value; undefined when there is any syntax error. */
  value: unknown
  errors: JsoncError[]
}

const errorMessages: Record<JsoncErrorCode, string> = {
  InvalidSymbol: "无法识别的符号。",
  InvalidNumberFormat: "数字格式无效。",
  PropertyNameExpected: "这里需要一个用双引号括起的字段名。",
  ValueExpected: "这里需要一个值。",
  ColonExpected: "字段名后需要冒号。",
  CommaExpected: "这里需要逗号。",
  CloseBraceExpected: "缺少右花括号 }。",
  CloseBracketExpected: "缺少右方括号 ]。",
  EndOfFileExpected: "文档在这里应当结束。",
  InvalidCommentToken: "注释格式无效。",
  UnexpectedEndOfComment: "块注释没有结束。",
  UnexpectedEndOfString: "字符串没有结束。",
  UnexpectedEndOfNumber: "数字没有结束。",
  InvalidUnicode: "Unicode 转义无效。",
  InvalidEscapeCharacter: "字符串中的转义无效。",
  InvalidCharacter: "字符串中不能直接包含控制字符。",
  DuplicateKey: "同一对象中字段名重复。",
  "<unknown ParseErrorCode>": "语法错误。",
}

const parseOptions = { disallowComments: false, allowTrailingComma: true, allowEmptyContent: false }

function duplicateKeys(node: JsoncNode, errors: JsoncError[]) {
  if (node.type === "object") {
    const seen = new Set<string>()
    for (const property of node.children ?? []) {
      const key = property.children?.[0]
      const name = propertyKey(property)
      if (key && name !== undefined) {
        if (seen.has(name)) {
          errors.push({ code: "DuplicateKey", offset: key.offset, length: key.length, message: errorMessages.DuplicateKey })
        }
        seen.add(name)
      }
    }
  }
  for (const child of node.children ?? []) duplicateKeys(child, errors)
}

export function parseJsonc(text: string): JsoncParse {
  const raw: ParseError[] = []
  const tree = parseTree(text, raw, parseOptions)
  const errors: JsoncError[] = raw.map((error) => {
    const code = printParseErrorCode(error.error)
    return { code, offset: error.offset, length: error.length, message: errorMessages[code] }
  })
  if (!tree && errors.length === 0) {
    errors.push({ code: "ValueExpected", offset: 0, length: 0, message: errorMessages.ValueExpected })
  }
  if (tree) duplicateKeys(tree, errors)
  if (!tree || errors.length > 0) {
    errors.sort((a, b) => a.offset - b.offset)
    return { tree: null, value: undefined, errors }
  }
  return { tree, value: nodeValue(tree), errors: [] }
}

/** Plain value of a node. Keys are defined, never assigned, so `__proto__` stays data. */
export function nodeValue(node: JsoncNode): unknown {
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

export type JsonPath = readonly (string | number)[]

/** Key of a property node. */
export function propertyKey(property: JsoncNode): string | undefined {
  const value: unknown = property.children?.[0]?.value
  return typeof value === "string" ? value : undefined
}

/** The value node at `path`, or undefined when any segment is missing. */
export function findNode(root: JsoncNode, path: JsonPath): JsoncNode | undefined {
  return findNodeAtLocation(root, [...path])
}

/** Deepest existing node along `path` and whether it is the full path. */
export function findNearest(root: JsoncNode, path: JsonPath): { node: JsoncNode; exact: boolean } {
  let node = root
  for (const segment of path) {
    const next = findNode(node, [segment])
    if (!next) return { node, exact: false }
    node = next
  }
  return { node, exact: true }
}

/** Property node (key + value) for `key` directly inside `object`. */
export function findProperty(object: JsoncNode, key: string): JsoncNode | undefined {
  if (object.type !== "object") return undefined
  return object.children?.find((property) => propertyKey(property) === key)
}

/** 1-based line and column (in UTF-16 code units, as editors count). */
export function lineColumn(text: string, offset: number): { line: number; column: number } {
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

// --- Patching ---------------------------------------------------------------

export type FormatOptions = { indent: string; eol: string }

export const defaultFormat: FormatOptions = { indent: "  ", eol: "\n" }

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)

const own = (value: Record<string, unknown>, key: string) => Object.prototype.hasOwnProperty.call(value, key)

export function jsonEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (Array.isArray(a) && Array.isArray(b)) {
    return a.length === b.length && a.every((item, index) => jsonEqual(item, b[index]))
  }
  if (isRecord(a) && isRecord(b)) {
    const keys = Object.keys(a)
    return keys.length === Object.keys(b).length && keys.every((key) => own(b, key) && jsonEqual(a[key], b[key]))
  }
  return false
}

type Operation = { path: (string | number)[]; value: unknown; insert: boolean }

/**
 * Operations turning `before` into `after`, deepest first. Within one array,
 * paired elements are diffed in place, surplus elements are removed from the
 * highest index down and new ones inserted in ascending order, so every
 * operation's path is still valid when it is applied.
 */
function operations(path: (string | number)[], before: unknown, after: unknown, out: Operation[]) {
  if (jsonEqual(before, after)) return
  if (isRecord(before) && isRecord(after)) {
    for (const key of Object.keys(before)) {
      if (own(after, key)) operations([...path, key], before[key], after[key], out)
      else out.push({ path: [...path, key], value: undefined, insert: false })
    }
    for (const key of Object.keys(after)) {
      if (!own(before, key)) out.push({ path: [...path, key], value: after[key], insert: false })
    }
    return
  }
  if (Array.isArray(before) && Array.isArray(after)) {
    let prefix = 0
    while (prefix < before.length && prefix < after.length && jsonEqual(before[prefix], after[prefix])) prefix += 1
    let suffix = 0
    while (
      suffix < before.length - prefix &&
      suffix < after.length - prefix &&
      jsonEqual(before[before.length - 1 - suffix], after[after.length - 1 - suffix])
    ) {
      suffix += 1
    }
    const oldMiddle = before.length - prefix - suffix
    const newMiddle = after.length - prefix - suffix
    const paired = Math.min(oldMiddle, newMiddle)
    for (let index = prefix; index < prefix + paired; index += 1) {
      operations([...path, index], before[index], after[index], out)
    }
    for (let index = prefix + oldMiddle - 1; index >= prefix + paired; index -= 1) {
      out.push({ path: [...path, index], value: undefined, insert: false })
    }
    for (let index = prefix + paired; index < prefix + newMiddle; index += 1) {
      out.push({ path: [...path, index], value: after[index], insert: true })
    }
    return
  }
  out.push({ path, value: after, insert: false })
}

const COMMA = 5 // SyntaxKind.CommaToken

/** Offset of the next non-trivia token at or after `from`, if it is a comma. */
function commaFrom(text: string, from: number): number | null {
  const scanner = createScanner(text, true)
  scanner.setPosition(from)
  scanner.scan()
  return scanner.getToken() === COMMA ? scanner.getTokenOffset() : null
}

/**
 * Removes the array element or object property at `path` together with one
 * separating comma. Unlike `modify`, it never deletes comments: only the
 * value, its comma, and a line left blank by the removal go away.
 */
function removeAt(text: string, path: readonly (string | number)[]): string {
  const tree = parseJsonc(text).tree
  if (!tree) throw new Error("cannot patch invalid JSONC")
  const parentPath = path.slice(0, -1)
  const last = path[path.length - 1]
  const parent = findNode(tree, parentPath)
  if (!parent || last === undefined) throw new Error("JSONC removal path not found")
  const children = parent.children ?? []
  const index =
    typeof last === "number" ? last : children.findIndex((property) => propertyKey(property) === last)
  const child = children[index]
  if (!child) throw new Error("JSONC removal path not found")
  const ranges: { from: number; to: number }[] = [{ from: child.offset, to: child.offset + child.length }]
  const after = commaFrom(text, child.offset + child.length)
  const previous = children[index - 1]
  if (after !== null) {
    let to = after + 1
    while (text[to] === " " || text[to] === "\t") to += 1
    if (/^\s*$/.test(text.slice(child.offset + child.length, after))) {
      ranges[0] = { from: child.offset, to }
    } else {
      ranges.push({ from: after, to })
    }
  } else if (previous) {
    const before = commaFrom(text, previous.offset + previous.length)
    if (before !== null && /^\s*$/.test(text.slice(before + 1, child.offset))) {
      ranges[0] = { from: before, to: child.offset + child.length }
    } else if (before !== null) {
      ranges.push({ from: before, to: before + 1 })
    }
  }
  ranges.sort((a, b) => b.from - a.from)
  let result = text
  for (const range of ranges) {
    result = result.slice(0, range.from) + result.slice(range.to)
    // Drop a line the removal left with only whitespace.
    let start = range.from
    while (start > 0 && (result[start - 1] === " " || result[start - 1] === "\t")) start -= 1
    let end = range.from
    while (end < result.length && (result[end] === " " || result[end] === "\t")) end += 1
    const lineStart = start === 0 || result[start - 1] === "\n"
    if (lineStart && result[end] === "\n") result = result.slice(0, start) + result.slice(end + 1)
    else if (lineStart && result[end] === "\r" && result[end + 1] === "\n") result = result.slice(0, start) + result.slice(end + 2)
  }
  return result
}

function formatting(format: FormatOptions): FormattingOptions {
  const tabs = format.indent.includes("\t")
  return { insertSpaces: !tabs, tabSize: tabs ? 1 : format.indent.length, eol: format.eol }
}

/**
 * Rewrites `text` (whose parsed value is `before`) so that it parses to
 * `after`, touching only the deepest differing nodes. Comments and formatting
 * outside replaced nodes are preserved. Throws when `text` is not valid JSONC
 * or does not parse to `before`: a visual edit must never be applied over a
 * buffer it was not derived from.
 */
export function patchJsonc(
  text: string,
  before: unknown,
  after: unknown,
  format: FormatOptions = defaultFormat
): string {
  const parsed = parseJsonc(text)
  if (!parsed.tree) throw new Error("cannot patch invalid JSONC")
  if (!jsonEqual(parsed.value, before)) throw new Error("JSONC text does not match the edited value")
  const out: Operation[] = []
  operations([], before, after, out)
  const formattingOptions = formatting(format)
  let result = text
  for (const operation of out) {
    if (operation.path.length === 0) return formatJsonc(after, format)
    if (operation.value === undefined) {
      result = removeAt(result, operation.path)
      continue
    }
    const edits = modify(result, operation.path, operation.value, {
      formattingOptions,
      isArrayInsertion: operation.insert,
    })
    result = applyJsoncEdits(result, edits)
  }
  const check = parseJsonc(result)
  if (!check.tree || !jsonEqual(check.value, after)) throw new Error("JSONC patch did not produce the edited value")
  return result
}

/** Canonical text for a value with no prior buffer. */
export function formatJsonc(value: unknown, format: FormatOptions = defaultFormat): string {
  return JSON.stringify(value, null, format.indent).split("\n").join(format.eol) + format.eol
}
