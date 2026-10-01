import {
  findNearest,
  findProperty,
  formatJsonc,
  jsonEqual,
  lineColumn,
  parseJsonc,
  patchJsonc,
  type JsonPath,
  type JsoncNode,
} from "./jsonc.ts"

// One canonical draft per policy, shared by the visual editor and the JSONC
// text editor. The JSONC buffer is the source of truth for what the
// administrator typed (comments included); `value` is the last buffer that
// passed both the JSONC parse and the generated runtime schema. Nothing here
// performs I/O: saving, and the CAS base it moves, stay with the page.

/** Structural subset of a generated Zod schema; the web never redefines DTOs. */
export type RuntimeSchema<T> = {
  safeParse(input: unknown):
    | { success: true; data: T }
    | { success: false; error: { issues: readonly SchemaIssueInput[] } }
}

export type SchemaIssueInput = {
  code: string
  path: readonly PropertyKey[]
  message: string
  keys?: readonly string[]
}

export type DocumentProblem = {
  /** syntax: JSONC; schema: generated runtime schema; semantic: editor rules. */
  source: "syntax" | "schema" | "semantic"
  /** Path in the policy value; empty for the document root. */
  path: (string | number)[]
  message: string
  /** Range in the text buffer, for highlighting. */
  from: number
  to: number
  line: number
  column: number
}

export type PolicyDocument<T> = {
  /** CAS base: only a successful save moves it. */
  base: { revision: number; value: T }
  text: string
  /** Last fully valid value (equals `base.value` before any edit). */
  value: T
  /** Text that produced `value`, for an explicit discard of invalid edits. */
  validText: string
  problems: DocumentProblem[]
  /** Increments on every edit, so late async results can be dropped. */
  seq: number
}

export function openDocument<T>(
  base: { revision: number; value: T },
  schema: RuntimeSchema<T>,
  text: string = formatJsonc(base.value)
): PolicyDocument<T> {
  const document: PolicyDocument<T> = {
    base,
    text,
    value: base.value,
    validText: formatJsonc(base.value),
    problems: [],
    seq: 0,
  }
  return evaluate(document, text, schema, 0)
}

/** True when the buffer parses and passes the runtime schema. */
export function isValid(document: PolicyDocument<unknown>): boolean {
  return document.problems.every((problem) => problem.source === "semantic")
}

function toPath(path: readonly PropertyKey[]): (string | number)[] {
  return path.filter((segment): segment is string | number => typeof segment !== "symbol")
}

function rangeFor(tree: JsoncNode, path: JsonPath, keyOf?: string): { from: number; to: number } {
  const { node } = findNearest(tree, path)
  if (keyOf !== undefined) {
    const key = findProperty(node, keyOf)?.children?.[0]
    if (key) return { from: key.offset, to: key.offset + key.length }
  }
  return { from: node.offset, to: node.offset + node.length }
}

function problemAt(
  text: string,
  source: DocumentProblem["source"],
  path: (string | number)[],
  message: string,
  range: { from: number; to: number }
): DocumentProblem {
  return { source, path, message, ...range, ...lineColumn(text, range.from) }
}

/** Maps runtime-schema issues to text ranges; unknown keys point at the key. */
export function schemaProblems(
  text: string,
  tree: JsoncNode,
  issues: readonly SchemaIssueInput[]
): DocumentProblem[] {
  return issues.flatMap((issue) => {
    const path = toPath(issue.path)
    if (issue.code === "unrecognized_keys" && issue.keys && issue.keys.length > 0) {
      return issue.keys.map((key) =>
        problemAt(text, "schema", [...path, key], `不支持的字段 "${key}"。`, rangeFor(tree, path, key))
      )
    }
    return [problemAt(text, "schema", path, issue.message, rangeFor(tree, path))]
  })
}

/** `grants[2].path[0]` → ["grants", 2, "path", 0]; editor and controller field names. */
export function parseFieldPath(field: string): (string | number)[] {
  const path: (string | number)[] = []
  for (const match of field.matchAll(/([^.[\]]+)|\[(\d+)\]/g)) {
    if (match[2] !== undefined) path.push(Number(match[2]))
    else if (match[1] !== undefined) path.push(match[1])
  }
  return path
}

/** Places editor-side (semantic) issues on the current buffer. */
export function semanticProblems(
  document: PolicyDocument<unknown>,
  issues: readonly { field: string; message: string }[]
): DocumentProblem[] {
  const tree = parseJsonc(document.text).tree
  return issues.map((issue) => {
    const path = parseFieldPath(issue.field)
    const range = tree ? rangeFor(tree, path) : { from: 0, to: 0 }
    return problemAt(document.text, "semantic", path, issue.message, range)
  })
}

function evaluate<T>(
  document: PolicyDocument<T>,
  text: string,
  schema: RuntimeSchema<T>,
  seq: number
): PolicyDocument<T> {
  const parsed = parseJsonc(text)
  if (!parsed.tree) {
    const problems = parsed.errors.map((error) =>
      problemAt(text, "syntax", [], error.message, { from: error.offset, to: error.offset + error.length })
    )
    return { ...document, text, problems, seq }
  }
  const result = schema.safeParse(parsed.value)
  if (!result.success) {
    return { ...document, text, problems: schemaProblems(text, parsed.tree, result.error.issues), seq }
  }
  return { ...document, text, value: result.data, validText: text, problems: [], seq }
}

/** The administrator typed in the JSONC editor. */
export function editText<T>(
  document: PolicyDocument<T>,
  text: string,
  schema: RuntimeSchema<T>
): PolicyDocument<T> {
  if (text === document.text) return document
  return evaluate(document, text, schema, document.seq + 1)
}

export type VisualEdit<T> =
  | { ok: true; document: PolicyDocument<T> }
  | { ok: false; reason: "invalid_buffer" | "invalid_value"; problems: DocumentProblem[] }

/**
 * A visual editor produced a new value. Refused while the buffer is invalid:
 * the visual editor is derived from `value`, and patching would silently drop
 * whatever the administrator is in the middle of typing. The edit is applied
 * as minimal text changes so comments outside the touched nodes survive.
 */
export function applyVisual<T>(
  document: PolicyDocument<T>,
  next: T,
  schema: RuntimeSchema<T>
): VisualEdit<T> {
  if (!isValid(document)) return { ok: false, reason: "invalid_buffer", problems: document.problems }
  if (jsonEqual(document.value, next)) return { ok: true, document }
  const text = patchJsonc(document.text, document.value, next)
  const updated = evaluate(document, text, schema, document.seq + 1)
  if (!isValid(updated)) return { ok: false, reason: "invalid_value", problems: updated.problems }
  return { ok: true, document: updated }
}

/** Explicit discard of text edits back to the last valid buffer. */
export function discardInvalidText<T>(document: PolicyDocument<T>, schema: RuntimeSchema<T>): PolicyDocument<T> {
  return evaluate(document, document.validText, schema, document.seq + 1)
}

/** Explicit revert of everything back to the CAS base. */
export function revertToBase<T>(document: PolicyDocument<T>, schema: RuntimeSchema<T>): PolicyDocument<T> {
  return { ...openDocument(document.base, schema), seq: document.seq + 1 }
}

export type DocumentChange = {
  /** The value differs from the base (what the controller would store). */
  valueChanged: boolean
  /** The buffer differs from a canonical rendering of the value (comments, layout, or invalid edits). */
  textOnly: boolean
}

export function documentChange(document: PolicyDocument<unknown>): DocumentChange {
  const valueChanged = !jsonEqual(document.value, document.base.value)
  return {
    valueChanged,
    textOnly: !valueChanged && document.text !== formatJsonc(document.base.value),
  }
}

/** Unsaved work worth keeping across navigation. */
export function hasUnsavedWork(document: PolicyDocument<unknown>): boolean {
  const change = documentChange(document)
  return change.valueChanged || change.textOnly || !isValid(document)
}

/**
 * A save of `sent` at `seq` succeeded with canonical `saved` at `revision`.
 * The base moves; the buffer is kept (with its comments) only when nothing
 * was edited since the request was sent. A later edit stays unsaved on the
 * new base and is compared against it.
 */
export function settleSaved<T>(
  document: PolicyDocument<T>,
  saved: { revision: number; value: T },
  sentSeq: number,
  schema: RuntimeSchema<T>
): PolicyDocument<T> {
  const rebased = { ...document, base: saved }
  if (document.seq !== sentSeq) return rebased
  if (jsonEqual(document.value, saved.value)) return rebased
  // The controller canonicalised the policy: show what it stored.
  return evaluate(rebased, patchJsonc(document.text, document.value, saved.value), schema, document.seq + 1)
}

/**
 * The controller moved on (save conflict or reload). The draft and its old
 * base stay untouched; the caller shows `remote` beside it and only an
 * explicit choice replaces the draft.
 */
export function remoteDiffers<T>(document: PolicyDocument<T>, remote: { revision: number; value: T }): boolean {
  return remote.revision !== document.base.revision || !jsonEqual(remote.value, document.base.value)
}

/** What a navigation-scoped registry keeps; everything else is recomputed. */
export type StoredDocument<T> = {
  base: { revision: number; value: T }
  text: string
  validText: string
  seq: number
}

export function storeDocument<T>(document: PolicyDocument<T>): StoredDocument<T> {
  return { base: document.base, text: document.text, validText: document.validText, seq: document.seq }
}

export function restoreDocument<T>(stored: StoredDocument<T>, schema: RuntimeSchema<T>): PolicyDocument<T> {
  const valid = openDocument(stored.base, schema, stored.validText)
  const document = isValid(valid) ? valid : openDocument(stored.base, schema)
  return evaluate({ ...document, validText: isValid(valid) ? stored.validText : document.validText }, stored.text, schema, stored.seq)
}
