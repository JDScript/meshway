import {
  applyEdits,
  createScanner,
  format,
  parseTree,
  printParseErrorCode,
  type Node,
  type ParseError,
} from "jsonc-parser"

// The configuration's JSONC source (JSON with comments and trailing commas)
// as the controller reads it, on top of `jsonc-parser`. The text is the
// draft and is never rebuilt from parsed data: these helpers only read it,
// place JSON Pointers on it, and reformat it when asked to.

export type JsoncNode = Node
export type JsonPath = readonly (string | number)[]

export type JsoncErrorCode =
  ReturnType<typeof printParseErrorCode> | "DuplicateKey" | "Empty"

export type JsoncError = {
  code: JsoncErrorCode
  offset: number
  length: number
}

export type JsoncParse =
  | { ok: true; tree: JsoncNode; value: unknown }
  | { ok: false; errors: JsoncError[] }

const OPTIONS = {
  disallowComments: false,
  allowTrailingComma: true,
  allowEmptyContent: false,
}

/** Duplicate keys are errors: last-wins would hide a typed value. */
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

/**
 * The segments of an RFC 6901 JSON Pointer (`/access/0/src`); the empty
 * pointer is the whole document. Null when it is not a pointer.
 */
export function pointerSegments(pointer: string): string[] | null {
  if (pointer === "") return []
  if (!pointer.startsWith("/")) return null
  return pointer
    .slice(1)
    .split("/")
    .map((segment) => segment.replaceAll("~1", "/").replaceAll("~0", "~"))
}

/** The pointer for `path` (the inverse of `pointerSegments`). */
export function pointerOf(path: JsonPath): string {
  return path
    .map(
      (segment) =>
        `/${String(segment).replaceAll("~", "~0").replaceAll("/", "~1")}`
    )
    .join("")
}

function child(node: JsoncNode, segment: string | number) {
  if (node.type === "array") {
    const index = typeof segment === "number" ? segment : Number(segment)
    if (!/^(0|[1-9][0-9]*)$/.test(String(segment))) return undefined
    return node.children?.[index]
  }
  if (node.type === "object") {
    const property = node.children?.find(
      (candidate) => propertyKey(candidate) === String(segment)
    )
    return property?.children?.[1]
  }
  return undefined
}

/** The deepest node that exists along `path`; `exact` when it is the whole path. */
export function findNearest(
  root: JsoncNode,
  path: JsonPath
): { node: JsoncNode; depth: number; exact: boolean } {
  let node = root
  let depth = 0
  for (const segment of path) {
    const next = child(node, segment)
    if (!next) return { node, depth, exact: false }
    node = next
    depth += 1
  }
  return { node, depth, exact: true }
}

/** The key node of property `key` of the object at `path`, when both exist. */
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

/** Where a range of text starts and ends. */
export type Span = { from: number; to: number }

/**
 * The text a JSON Pointer names: the value at it, or (when the text does
 * not hold the whole path) the opening of the deepest value that holds it.
 */
export function spanOfPointer(
  root: JsoncNode,
  pointer: string
): (Span & { exact: boolean }) | null {
  const segments = pointerSegments(pointer)
  if (segments === null) return null
  const { node, exact } = findNearest(root, segments)
  return exact
    ? { from: node.offset, to: node.offset + node.length, exact }
    : { from: node.offset, to: node.offset + 1, exact }
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
const LINE_COMMENT = 12
const BLOCK_COMMENT = 13
const EOF = 17

/** Every comment's range (the JSON grammar has none, so the editor marks them from this). */
export function commentRanges(text: string): Span[] {
  const scanner = createScanner(text, false)
  const ranges: Span[] = []
  for (let token = scanner.scan(); token !== EOF; token = scanner.scan()) {
    if (token === LINE_COMMENT || token === BLOCK_COMMENT) {
      const from = scanner.getTokenOffset()
      ranges.push({ from, to: from + scanner.getTokenLength() })
    }
  }
  return ranges
}

/**
 * The text laid out one value per line, two spaces deep. Comments and
 * values stay as written; only whitespace and line breaks change.
 */
export function formatJsonc(text: string): string {
  const eol = text.includes("\r\n") ? "\r\n" : "\n"
  const edits = format(text, undefined, {
    tabSize: 2,
    insertSpaces: true,
    eol,
    insertFinalNewline: true,
    keepLines: false,
  })
  return applyEdits(text, edits)
}
