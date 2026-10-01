// Schema-driven completion for the JSONC policy editors. Pure functions over
// text + JSON Schema (generated from the controller OpenAPI), independent of
// the code-editor package that renders them. The location scanner tolerates
// incomplete text, because completion is requested while the administrator is
// still typing and the buffer is usually invalid.

export type JsonSegment = string | number

export type JsoncLocation =
  | { kind: "none" }
  | {
      kind: "key"
      /** Path of the object whose key is being typed. */
      path: JsonSegment[]
      /** Keys already present before the cursor in that object. */
      present: string[]
      /** Text range to replace (the partial key, quotes included). */
      from: number
      to: number
      prefix: string
    }
  | {
      kind: "value"
      /** Path of the value being typed. */
      path: JsonSegment[]
      from: number
      to: number
      prefix: string
    }

type Frame =
  | { type: "object"; key: string | null; expectKey: boolean; afterValue: boolean; keys: string[] }
  | { type: "array"; index: number; afterValue: boolean }

function framePath(frames: readonly Frame[]): JsonSegment[] {
  const path: JsonSegment[] = []
  frames.forEach((frame, index) => {
    if (index === frames.length - 1) return
    if (frame.type === "object") path.push(frame.key ?? "")
    else path.push(frame.index)
  })
  return path
}

function stringEnd(text: string, start: number): { end: number; closed: boolean } {
  let index = start + 1
  while (index < text.length) {
    const char = text[index]
    if (char === "\\") {
      index += 2
      continue
    }
    if (char === '"') return { end: index + 1, closed: true }
    if (char === "\n" || char === "\r") return { end: index, closed: false }
    index += 1
  }
  return { end: text.length, closed: false }
}

function decode(raw: string): string {
  const body = raw.endsWith('"') && raw.length > 1 ? raw.slice(1, -1) : raw.slice(1)
  try {
    const value: unknown = JSON.parse(`"${body}"`)
    return typeof value === "string" ? value : body
  } catch {
    return body
  }
}

/** Where the cursor at `offset` is: typing a key, a value, or neither. */
export function locationAt(text: string, offset: number): JsoncLocation {
  const frames: Frame[] = []
  let pos = 0
  const top = () => frames[frames.length - 1]
  const consumed = () => {
    const frame = top()
    if (frame) frame.afterValue = true
  }
  const here = (from: number, to: number, prefix: string): JsoncLocation => {
    const frame = top()
    if (!frame) return { kind: "none" }
    if (frame.type === "object") {
      if (frame.expectKey) {
        return { kind: "key", path: framePath(frames), present: [...frame.keys], from, to, prefix }
      }
      if (frame.afterValue) return { kind: "none" }
      return { kind: "value", path: [...framePath(frames), frame.key ?? ""], from, to, prefix }
    }
    if (frame.afterValue) return { kind: "none" }
    return { kind: "value", path: [...framePath(frames), frame.index], from, to, prefix }
  }

  while (pos < offset) {
    const char = text[pos]
    if (char === undefined) break
    if (char === " " || char === "\t" || char === "\n" || char === "\r") {
      pos += 1
      continue
    }
    if (char === "/" && text[pos + 1] === "/") {
      let end = pos
      while (end < text.length && text[end] !== "\n" && text[end] !== "\r") end += 1
      if (offset <= end) return { kind: "none" }
      pos = end
      continue
    }
    if (char === "/" && text[pos + 1] === "*") {
      const close = text.indexOf("*/", pos + 2)
      const end = close === -1 ? text.length : close + 2
      if (offset < end || close === -1) return { kind: "none" }
      pos = end
      continue
    }
    if (char === "{") {
      frames.push({ type: "object", key: null, expectKey: true, afterValue: false, keys: [] })
      pos += 1
      continue
    }
    if (char === "[") {
      frames.push({ type: "array", index: 0, afterValue: false })
      pos += 1
      continue
    }
    if (char === "}" || char === "]") {
      frames.pop()
      consumed()
      pos += 1
      continue
    }
    if (char === ",") {
      const frame = top()
      if (frame?.type === "object") {
        frame.expectKey = true
        frame.key = null
        frame.afterValue = false
      } else if (frame?.type === "array") {
        frame.index += 1
        frame.afterValue = false
      }
      pos += 1
      continue
    }
    if (char === ":") {
      const frame = top()
      if (frame?.type === "object") {
        frame.expectKey = false
        frame.afterValue = false
      }
      pos += 1
      continue
    }
    if (char === '"') {
      const { end, closed } = stringEnd(text, pos)
      // The cursor is inside this string (or right after an unclosed one).
      if (offset < end || (!closed && offset === end)) {
        return here(pos, end, decode(text.slice(pos, offset)))
      }
      const frame = top()
      if (frame?.type === "object" && frame.expectKey) {
        frame.key = decode(text.slice(pos, end))
        frame.keys.push(frame.key)
        frame.expectKey = false
        frame.afterValue = true
      } else {
        consumed()
      }
      pos = end
      continue
    }
    let end = pos
    while (end < text.length && !/[\s,:[\]{}"/]/.test(text[end] ?? "")) end += 1
    if (end === pos) end += 1
    if (offset <= end) return here(pos, end, text.slice(pos, offset))
    consumed()
    pos = end
  }
  return here(offset, offset, "")
}

// --- JSON Schema navigation ---------------------------------------------------

/** Only the JSON Schema keywords completion uses. Everything is optional. */
export type SchemaView = {
  type?: string | string[]
  properties?: Record<string, unknown>
  required?: string[]
  items?: unknown
  enum?: unknown[]
  const?: unknown
  description?: string
  anyOf?: unknown[]
  oneOf?: unknown[]
  allOf?: unknown[]
  $ref?: string
  /** Controller extension: which inventory a string refers to. */
  "x-meshway-ref"?: string
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)

function view(value: unknown): SchemaView {
  if (!isRecord(value)) return {}
  const result: SchemaView = {}
  const { type, properties, required, items, description, $ref, anyOf, oneOf, allOf } = value
  if (typeof type === "string" || (Array.isArray(type) && type.every((item) => typeof item === "string"))) {
    result.type = type
  }
  if (isRecord(properties)) result.properties = properties
  if (Array.isArray(required)) result.required = required.filter((item) => typeof item === "string")
  if (items !== undefined) result.items = items
  if (Array.isArray(value.enum)) result.enum = value.enum
  if ("const" in value) result.const = value.const
  if (typeof description === "string") result.description = description
  if (typeof $ref === "string") result.$ref = $ref
  if (Array.isArray(anyOf)) result.anyOf = anyOf
  if (Array.isArray(oneOf)) result.oneOf = oneOf
  if (Array.isArray(allOf)) result.allOf = allOf
  const ref = value["x-meshway-ref"]
  if (typeof ref === "string") result["x-meshway-ref"] = ref
  return result
}

function resolvePointer(root: unknown, ref: string): unknown {
  if (!ref.startsWith("#")) return undefined
  let node: unknown = root
  for (const raw of ref.slice(1).split("/").filter((part) => part !== "")) {
    const part = raw.replaceAll("~1", "/").replaceAll("~0", "~")
    if (!isRecord(node)) return undefined
    node = node[part]
  }
  return node
}

/** Flattens `$ref` and unions into the alternatives a position may take. */
function alternatives(root: unknown, schema: unknown, depth = 0): SchemaView[] {
  if (depth > 16) return []
  const current = view(schema)
  if (current.$ref) return alternatives(root, resolvePointer(root, current.$ref), depth + 1)
  const branches = [...(current.anyOf ?? []), ...(current.oneOf ?? []), ...(current.allOf ?? [])]
  if (branches.length > 0) {
    const own = { ...current, anyOf: undefined, oneOf: undefined, allOf: undefined }
    return [own, ...branches.flatMap((branch) => alternatives(root, branch, depth + 1))]
  }
  return [current]
}

/** Schema alternatives at `path`, starting from `schema` inside document `root`. */
export function schemasAt(root: unknown, schema: unknown, path: readonly JsonSegment[]): SchemaView[] {
  let current = alternatives(root, schema)
  for (const segment of path) {
    current = current.flatMap((option) => {
      if (typeof segment === "string" && option.properties && segment in option.properties) {
        return alternatives(root, option.properties[segment])
      }
      if (typeof segment === "number" && option.items !== undefined) return alternatives(root, option.items)
      return []
    })
  }
  return current
}

// --- Completion -----------------------------------------------------------------

export type CompletionItem = {
  label: string
  /** Text inserted over [from, to). */
  insert: string
  detail?: string
  kind: "property" | "enum" | "literal" | "reference"
}

export type Completion = { from: number; to: number; items: CompletionItem[] }

/**
 * Inventory-backed suggestions for strings marked with `x-meshway-ref`
 * (for example node IDs shown with their display names). Keyed by the
 * extension value; the caller supplies only what the controller returned.
 */
export type ReferenceSuggestions = Record<string, readonly { value: string; label: string; detail?: string }[]>

const typesOf = (option: SchemaView) =>
  option.type === undefined ? [] : Array.isArray(option.type) ? option.type : [option.type]

function defaultFor(root: unknown, schema: unknown): string {
  const [first] = alternatives(root, schema)
  const types = first ? typesOf(first) : []
  if (first?.enum && first.enum.length > 0) return JSON.stringify(first.enum[0])
  if (types.includes("object")) return "{}"
  if (types.includes("array")) return "[]"
  if (types.includes("string")) return '""'
  if (types.includes("boolean")) return "false"
  if (types.includes("integer") || types.includes("number")) return "0"
  return "null"
}

export function completeAt(
  text: string,
  offset: number,
  root: unknown,
  schema: unknown,
  references: ReferenceSuggestions = {}
): Completion | null {
  const location = locationAt(text, offset)
  if (location.kind === "none") return null
  const prefix = location.prefix.replace(/^"/, "").toLowerCase()
  const matches = (label: string) => label.toLowerCase().startsWith(prefix)

  if (location.kind === "key") {
    const seen = new Set<string>()
    const items: CompletionItem[] = []
    for (const option of schemasAt(root, schema, location.path)) {
      for (const [key, property] of Object.entries(option.properties ?? {})) {
        if (seen.has(key) || location.present.includes(key) || !matches(key)) continue
        seen.add(key)
        const described = alternatives(root, property).find((candidate) => candidate.description)
        items.push({
          label: key,
          insert: `${JSON.stringify(key)}: ${defaultFor(root, property)}`,
          detail: described?.description ?? (option.required?.includes(key) ? "必填" : undefined),
          kind: "property",
        })
      }
    }
    return { from: location.from, to: location.to, items }
  }

  const options = schemasAt(root, schema, location.path)
  const items: CompletionItem[] = []
  const seen = new Set<string>()
  const push = (item: CompletionItem) => {
    if (seen.has(item.insert)) return
    seen.add(item.insert)
    items.push(item)
  }
  for (const option of options) {
    for (const value of option.enum ?? []) {
      if (matches(String(value))) push({ label: String(value), insert: JSON.stringify(value), kind: "enum" })
    }
    if (option.const !== undefined && matches(String(option.const))) {
      push({ label: String(option.const), insert: JSON.stringify(option.const), kind: "enum" })
    }
    const types = typesOf(option)
    if (types.includes("boolean")) {
      for (const value of ["true", "false"]) if (matches(value)) push({ label: value, insert: value, kind: "literal" })
    }
    if (types.includes("null") && matches("null")) push({ label: "null", insert: "null", kind: "literal" })
    const ref = option["x-meshway-ref"]
    for (const suggestion of ref ? (references[ref] ?? []) : []) {
      if (matches(suggestion.value) || matches(suggestion.label)) {
        push({
          label: suggestion.label,
          insert: JSON.stringify(suggestion.value),
          detail: suggestion.detail ?? suggestion.value,
          kind: "reference",
        })
      }
    }
  }
  return { from: location.from, to: location.to, items }
}
