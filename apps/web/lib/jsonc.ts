// Minimal JSONC (JSON with comments and trailing commas) parser and
// comment-preserving patcher for the policy text editors.
//
// The tree mirrors the `Node` shape of `jsonc-parser`'s `parseTree` (type,
// offset, length, colonOffset, children, value) so the parser can be replaced
// by that package once it is an approved dependency without touching callers.
// The parser is strict about everything except comments and trailing commas:
// it stops at the first syntax error and then returns no tree.

export type JsoncNodeType = "object" | "array" | "property" | "string" | "number" | "boolean" | "null"

export type JsoncNode = {
  type: JsoncNodeType
  offset: number
  length: number
  /** Property nodes only: offset of the ':'. */
  colonOffset?: number
  /** Objects: property nodes. Arrays: elements. Properties: [key, value]. */
  children?: JsoncNode[]
  /** Scalars and property keys only. */
  value?: string | number | boolean | null
}

export type JsoncErrorCode =
  | "InvalidSymbol"
  | "InvalidNumberFormat"
  | "PropertyNameExpected"
  | "ValueExpected"
  | "ColonExpected"
  | "CommaExpected"
  | "CloseBraceExpected"
  | "CloseBracketExpected"
  | "EndOfFileExpected"
  | "UnexpectedEndOfComment"
  | "UnexpectedEndOfString"
  | "InvalidEscapeCharacter"
  | "InvalidCharacter"
  | "DuplicateKey"

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
  UnexpectedEndOfComment: "块注释没有结束。",
  UnexpectedEndOfString: "字符串没有结束。",
  InvalidEscapeCharacter: "字符串中的转义无效。",
  InvalidCharacter: "字符串中不能直接包含控制字符。",
  DuplicateKey: "同一对象中字段名重复。",
}

class Failure extends Error {
  readonly detail: JsoncError

  constructor(detail: JsoncError) {
    super(detail.message)
    this.detail = detail
  }
}

function fail(code: JsoncErrorCode, offset: number, length = 1): never {
  throw new Failure({ code, offset, length: Math.max(length, 0), message: errorMessages[code] })
}

const end = (node: JsoncNode) => node.offset + node.length

const isDigit = (char: string | undefined) => char !== undefined && char >= "0" && char <= "9"

class Parser {
  pos = 0
  readonly text: string

  constructor(text: string) {
    this.text = text
  }

  /** Skips whitespace and comments. */
  skip() {
    const { text } = this
    while (this.pos < text.length) {
      const char = text[this.pos]
      if (char === " " || char === "\t" || char === "\n" || char === "\r" || char === "\uFEFF") {
        this.pos += 1
      } else if (char === "/" && text[this.pos + 1] === "/") {
        while (this.pos < text.length && text[this.pos] !== "\n" && text[this.pos] !== "\r") this.pos += 1
      } else if (char === "/" && text[this.pos + 1] === "*") {
        const end = text.indexOf("*/", this.pos + 2)
        if (end === -1) fail("UnexpectedEndOfComment", this.pos, text.length - this.pos)
        this.pos = end + 2
      } else {
        return
      }
    }
  }

  value(): JsoncNode {
    this.skip()
    const char = this.text[this.pos]
    if (char === "{") return this.object()
    if (char === "[") return this.array()
    if (char === '"') return this.string()
    if (char === "-" || isDigit(char)) return this.number()
    for (const [word, value] of [
      ["true", true],
      ["false", false],
      ["null", null],
    ] as const) {
      if (this.text.startsWith(word, this.pos)) {
        const offset = this.pos
        this.pos += word.length
        return { type: value === null ? "null" : "boolean", offset, length: word.length, value }
      }
    }
    fail(char === undefined ? "ValueExpected" : "InvalidSymbol", this.pos)
  }

  object(): JsoncNode {
    const node: JsoncNode = { type: "object", offset: this.pos, length: 0, children: [] }
    const keys = new Set<string>()
    this.pos += 1
    this.skip()
    while (this.text[this.pos] !== "}") {
      if (this.pos >= this.text.length) fail("CloseBraceExpected", this.pos, 0)
      if (this.text[this.pos] !== '"') fail("PropertyNameExpected", this.pos)
      const key = this.string()
      const name = String(key.value)
      if (keys.has(name)) fail("DuplicateKey", key.offset, key.length)
      keys.add(name)
      this.skip()
      if (this.text[this.pos] !== ":") fail("ColonExpected", this.pos, 0)
      const colonOffset = this.pos
      this.pos += 1
      const value = this.value()
      node.children?.push({
        type: "property",
        offset: key.offset,
        length: value.offset + value.length - key.offset,
        colonOffset,
        children: [key, value],
      })
      this.skip()
      if (this.text[this.pos] === ",") {
        this.pos += 1
        this.skip()
      } else if (this.text[this.pos] !== "}") {
        // Point at where the separator is missing, not at the next token.
        fail(this.pos >= this.text.length ? "CloseBraceExpected" : "CommaExpected", end(value), 0)
      }
    }
    this.pos += 1
    node.length = this.pos - node.offset
    return node
  }

  array(): JsoncNode {
    const node: JsoncNode = { type: "array", offset: this.pos, length: 0, children: [] }
    this.pos += 1
    this.skip()
    while (this.text[this.pos] !== "]") {
      if (this.pos >= this.text.length) fail("CloseBracketExpected", this.pos, 0)
      const item = this.value()
      node.children?.push(item)
      this.skip()
      if (this.text[this.pos] === ",") {
        this.pos += 1
        this.skip()
      } else if (this.text[this.pos] !== "]") {
        fail(this.pos >= this.text.length ? "CloseBracketExpected" : "CommaExpected", end(item), 0)
      }
    }
    this.pos += 1
    node.length = this.pos - node.offset
    return node
  }

  string(): JsoncNode {
    const { text } = this
    const offset = this.pos
    let end = offset + 1
    while (true) {
      const char = text[end]
      if (char === undefined || char === "\n" || char === "\r") {
        fail("UnexpectedEndOfString", offset, end - offset)
      }
      if (char === '"') break
      if (char === "\\") {
        const next = text[end + 1]
        if (next === "u") {
          if (!/^[0-9a-fA-F]{4}$/.test(text.slice(end + 2, end + 6))) fail("InvalidEscapeCharacter", end, 2)
          end += 6
          continue
        }
        if (next === undefined || !'"\\/bfnrt'.includes(next)) fail("InvalidEscapeCharacter", end, 2)
        end += 2
        continue
      }
      if (char < " ") fail("InvalidCharacter", end)
      end += 1
    }
    this.pos = end + 1
    const value: unknown = JSON.parse(text.slice(offset, this.pos))
    return { type: "string", offset, length: this.pos - offset, value: String(value) }
  }

  number(): JsoncNode {
    const match = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/.exec(this.text.slice(this.pos))
    const offset = this.pos
    if (!match) fail("InvalidNumberFormat", offset)
    this.pos += match[0].length
    if (/[0-9A-Za-z.]/.test(this.text[this.pos] ?? "")) fail("InvalidNumberFormat", offset, this.pos - offset + 1)
    return { type: "number", offset, length: match[0].length, value: Number(match[0]) }
  }
}

export function parseJsonc(text: string): JsoncParse {
  const parser = new Parser(text)
  try {
    const tree = parser.value()
    parser.skip()
    if (parser.pos < text.length) fail("EndOfFileExpected", parser.pos, text.length - parser.pos)
    return { tree, value: nodeValue(tree), errors: [] }
  } catch (error) {
    if (error instanceof Failure) return { tree: null, value: undefined, errors: [error.detail] }
    throw error
  }
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
    default:
      return node.value
  }
}

export type JsonPath = readonly (string | number)[]

/** Key of a property node. */
export function propertyKey(property: JsoncNode): string | undefined {
  const value = property.children?.[0]?.value
  return typeof value === "string" ? value : undefined
}

/** The value node at `path`, or undefined when any segment is missing. */
export function findNode(root: JsoncNode, path: JsonPath): JsoncNode | undefined {
  let node: JsoncNode | undefined = root
  for (const segment of path) {
    if (!node) return undefined
    if (node.type === "object" && typeof segment === "string") {
      node = node.children?.find((property) => propertyKey(property) === segment)?.children?.[1]
    } else if (node.type === "array" && typeof segment === "number") {
      node = node.children?.[segment]
    } else {
      return undefined
    }
  }
  return node
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

export type TextEdit = { offset: number; length: number; content: string }

export type FormatOptions = { indent: string; eol: string }

export const defaultFormat: FormatOptions = { indent: "  ", eol: "\n" }

/** Applies non-overlapping edits. */
export function applyEdits(text: string, edits: readonly TextEdit[]): string {
  const sorted = [...edits].sort((a, b) => b.offset - a.offset || b.length - a.length)
  let result = text
  let limit = text.length
  for (const edit of sorted) {
    if (edit.offset + edit.length > limit) throw new Error("overlapping JSONC edits")
    result = result.slice(0, edit.offset) + edit.content + result.slice(edit.offset + edit.length)
    limit = edit.offset
  }
  return result
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)

export function jsonEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (Array.isArray(a) && Array.isArray(b)) {
    return a.length === b.length && a.every((item, index) => jsonEqual(item, b[index]))
  }
  if (isRecord(a) && isRecord(b)) {
    const keys = Object.keys(a)
    return (
      keys.length === Object.keys(b).length &&
      keys.every((key) => Object.prototype.hasOwnProperty.call(b, key) && jsonEqual(a[key], b[key]))
    )
  }
  return false
}

function lineIndent(text: string, offset: number): string {
  let start = offset
  while (start > 0 && text[start - 1] !== "\n" && text[start - 1] !== "\r") start -= 1
  return /^[ \t]*/.exec(text.slice(start))?.[0] ?? ""
}

/** Indentation of a container's members (existing, or one level deeper). */
function childIndent(text: string, container: JsoncNode, format: FormatOptions): string {
  const first = container.children?.[0]
  return first ? lineIndent(text, first.offset) : lineIndent(text, container.offset) + format.indent
}

function render(value: unknown, indent: string, format: FormatOptions): string {
  return JSON.stringify(value, null, format.indent).split("\n").join(format.eol + indent)
}

/** Offset just after the separating comma that follows `from`, or null. */
function commaAfter(text: string, from: number): number | null {
  const parser = new Parser(text)
  parser.pos = from
  try {
    parser.skip()
  } catch {
    return null
  }
  return text[parser.pos] === "," ? parser.pos + 1 : null
}

/** Removal edits for the given child indices, one per contiguous run. */
function removals(text: string, children: readonly JsoncNode[], indices: readonly number[]): TextEdit[] {
  const sorted = [...indices].sort((a, b) => a - b)
  const edits: TextEdit[] = []
  let index = 0
  while (index < sorted.length) {
    const first = sorted[index]
    if (first === undefined) break
    let last = first
    while (sorted[index + 1] === last + 1) {
      index += 1
      last += 1
    }
    edits.push(removal(text, children, first, last))
    index += 1
  }
  return edits
}

/** Removal range for children `first..=last`, keeping neighbouring comments. */
function removal(text: string, children: readonly JsoncNode[], first: number, last: number): TextEdit {
  const child = children[last]
  const head = children[first]
  if (!child || !head) throw new Error("JSONC removal out of range")
  const next = children[last + 1]
  const previous = children[first - 1]
  if (next) {
    const comma = commaAfter(text, end(child)) ?? end(child)
    let stop = comma
    while (text[stop] === " " || text[stop] === "\t") stop += 1
    if (text[stop] === "\r") stop += 1
    if (text[stop] === "\n") stop += 1
    // Also drop the now-empty indentation of the removed line.
    let start = head.offset
    while (start > 0 && (text[start - 1] === " " || text[start - 1] === "\t")) start -= 1
    if (start > 0 && text[start - 1] !== "\n" && text[start - 1] !== "\r") start = head.offset
    if (stop > next.offset) stop = next.offset
    return { offset: start, length: stop - start, content: "" }
  }
  if (previous) return { offset: end(previous), length: end(child) - end(previous), content: "" }
  const comma = commaAfter(text, end(child))
  return { offset: head.offset, length: (comma ?? end(child)) - head.offset, content: "" }
}

function insertion(
  text: string,
  container: JsoncNode,
  before: JsoncNode | undefined,
  items: readonly string[],
  format: FormatOptions
): TextEdit {
  const children = container.children ?? []
  const outer = lineIndent(text, container.offset)
  const inner = childIndent(text, container, format)
  const sep = `,${format.eol}${inner}`
  if (before) {
    return { offset: before.offset, length: 0, content: items.join(sep) + sep }
  }
  const last = children[children.length - 1]
  if (last) return { offset: end(last), length: 0, content: sep + items.join(sep) }
  return {
    offset: container.offset + 1,
    length: 0,
    content: format.eol + inner + items.join(sep) + format.eol + outer,
  }
}

function diff(
  text: string,
  node: JsoncNode,
  before: unknown,
  after: unknown,
  format: FormatOptions,
  edits: TextEdit[]
) {
  if (jsonEqual(before, after)) return
  if (node.type === "object" && isRecord(before) && isRecord(after)) {
    const properties = node.children ?? []
    const removed = properties.flatMap((property, index) =>
      Object.prototype.hasOwnProperty.call(after, propertyKey(property) ?? "") ? [] : [index]
    )
    if (removed.length > 0 && removed.length === properties.length) {
      replace(text, node, after, format, edits)
      return
    }
    const added = Object.keys(after).filter((key) => !Object.prototype.hasOwnProperty.call(before, key))
    // Removing the last property and appending would edit the same separator.
    if (removed.length > 0 && added.length > 0 && removed.includes(properties.length - 1)) {
      replace(text, node, after, format, edits)
      return
    }
    for (const property of properties) {
      const key = propertyKey(property)
      const value = property.children?.[1]
      if (key !== undefined && value && Object.prototype.hasOwnProperty.call(after, key)) {
        diff(text, value, before[key], after[key], format, edits)
      }
    }
    edits.push(...removals(text, properties, removed))
    if (added.length > 0) {
      const indent = childIndent(text, node, format)
      const items = added.map((key) => `${JSON.stringify(key)}: ${render(after[key], indent, format)}`)
      edits.push(insertion(text, node, undefined, items, format))
    }
    return
  }
  if (node.type === "array" && Array.isArray(before) && Array.isArray(after)) {
    const children = node.children ?? []
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
      const child = children[index]
      if (child) diff(text, child, before[index], after[index], format, edits)
    }
    if (oldMiddle > newMiddle) {
      if (newMiddle === 0 && after.length === 0) {
        replace(text, node, after, format, edits)
        return
      }
      edits.push(removal(text, children, prefix + paired, prefix + oldMiddle - 1))
    } else if (newMiddle > oldMiddle) {
      const indent = childIndent(text, node, format)
      const items = after.slice(prefix + paired, prefix + newMiddle).map((item) => render(item, indent, format))
      edits.push(insertion(text, node, children[prefix + oldMiddle], items, format))
    }
    return
  }
  replace(text, node, after, format, edits)
}

function replace(text: string, node: JsoncNode, value: unknown, format: FormatOptions, edits: TextEdit[]) {
  edits.push({ offset: node.offset, length: node.length, content: render(value, lineIndent(text, node.offset), format) })
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
  const edits: TextEdit[] = []
  diff(text, parsed.tree, before, after, format, edits)
  return applyEdits(text, edits)
}

/** Canonical text for a value with no prior buffer. */
export function formatJsonc(value: unknown, format: FormatOptions = defaultFormat): string {
  return render(value, "", format) + format.eol
}
