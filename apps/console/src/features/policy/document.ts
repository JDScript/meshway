import type { Diagnostic, NetworkPolicy } from "@meshway/control-client"
import { zNetworkPolicy } from "@meshway/control-client/schemas"

import {
  findKey,
  findNearest,
  formatJsonc,
  moveJsonc,
  parseJsonc,
  patchJsonc,
  type JsoncErrorCode,
  type JsoncNode,
  type JsonPath,
} from "./jsonc.ts"

// The policy as it is edited: the JSONC text is the draft, and every view
// reads it through `readDocument`. Graphical edits go through `editPolicy`
// and `movePolicy`, which change the text in place and refuse to touch text
// that is not a valid policy, so an invalid source is never rewritten.

/** Where in the text a problem sits. */
export type Span = { from: number; to: number }

export type SchemaCode =
  "type" | "unknown_key" | "value" | "union" | "missing" | "other"

export type Problem =
  | ({ kind: "syntax"; code: JsoncErrorCode } & Span)
  | ({ kind: "schema"; code: SchemaCode; path: JsonPath; key?: string } & Span)
  | ({ kind: "id"; code: "syntax" | "duplicate"; path: JsonPath } & Span)

export type PolicyDocument = {
  text: string
  tree: JsoncNode | null
  /** In text order; the document can be saved when there are none. */
  problems: Problem[]
  /**
   * The policy the text holds; null when it does not parse or does not
   * match the schema. Rule-ID problems leave it set, so the graphical views
   * stay while an ID is being typed.
   */
  policy: NetworkPolicy | null
}

/** The sections of the document that hold rules with IDs shared across them. */
const SHARED_ID_SECTIONS = ["connections", "grants", "mesh"] as const
const RULE_ID = /^[A-Za-z0-9._-]{1,64}$/

export function readDocument(text: string): PolicyDocument {
  const parsed = parseJsonc(text)
  if (!parsed.ok)
    return {
      text,
      tree: null,
      policy: null,
      problems: parsed.errors.map((error) => ({
        kind: "syntax",
        code: error.code,
        from: error.offset,
        to: error.offset + error.length,
      })),
    }
  const tree = parsed.tree
  const result = zNetworkPolicy.safeParse(parsed.value)
  if (!result.success) {
    const problems: Problem[] = result.error.issues.map((issue) => {
      const path = issue.path.filter(
        (segment): segment is string | number => typeof segment !== "symbol"
      )
      if (issue.code === "unrecognized_keys") {
        const key = issue.keys[0] ?? ""
        const node = findKey(tree, path, key)
        return {
          kind: "schema",
          code: "unknown_key",
          path,
          key,
          ...spanOf(node ?? findNearest(tree, path).node),
        }
      }
      const { node, exact } = findNearest(tree, path)
      const code: SchemaCode = !exact
        ? "missing"
        : issue.code === "invalid_type"
          ? "type"
          : issue.code === "invalid_value"
            ? "value"
            : issue.code === "invalid_union"
              ? "union"
              : "other"
      return { kind: "schema", code, path, ...spanOf(node, !exact) }
    })
    return { text, tree, policy: null, problems: ordered(problems) }
  }
  const policy = result.data
  return { text, tree, policy, problems: idProblems(policy, tree) }
}

function spanOf(node: JsoncNode, opening = false): Span {
  // A missing field is marked at the opening of the object that lacks it.
  return opening
    ? { from: node.offset, to: node.offset + 1 }
    : { from: node.offset, to: node.offset + node.length }
}

const ordered = (problems: Problem[]) =>
  [...problems].sort((a, b) => a.from - b.from)

/** Rule IDs the controller would refuse: 1–64 of `A–Z a–z 0–9 . _ -`, unique across the first three layers. */
function idProblems(policy: NetworkPolicy, tree: JsoncNode): Problem[] {
  const seen = new Set<string>()
  const problems: Problem[] = []
  for (const section of SHARED_ID_SECTIONS) {
    policy[section].forEach((rule, index) => {
      const path = [section, index, "id"]
      const code = !RULE_ID.test(rule.id)
        ? "syntax"
        : seen.has(rule.id)
          ? "duplicate"
          : null
      seen.add(rule.id)
      if (code)
        problems.push({
          kind: "id",
          code,
          path,
          ...spanOf(findNearest(tree, path).node),
        })
    })
  }
  return ordered(problems)
}

/**
 * Applies a graphical edit to the text. Returns the text unchanged when it
 * is not a valid policy (the views then show the source as invalid) or when
 * the edit cannot be applied exactly.
 */
export function editPolicy(
  text: string,
  update: (policy: NetworkPolicy) => NetworkPolicy
): string {
  const document = readDocument(text)
  if (!document.policy) return text
  try {
    return patchJsonc(text, update(document.policy))
  } catch {
    return text
  }
}

/** Moves a rule of an ordered list one place up or down, with the comments above it. */
export function movePolicy(
  text: string,
  path: JsonPath,
  index: number,
  delta: -1 | 1
): string {
  if (!readDocument(text).policy) return text
  try {
    return moveJsonc(text, path, index, delta)
  } catch {
    return text
  }
}

/** Sections and rule fields in reading order. */
const KEY_ORDER = [
  "connections",
  "grants",
  "mesh",
  "public_rules",
  "profiles",
  "automatic",
  "assignments",
  "id",
  "kind",
  "src",
  "a",
  "b",
  "dst",
  "destinations",
  "value",
  "action",
  "target",
  "candidates",
  "via",
  "bidirectional",
  "rules",
  "fallback",
  "regions",
  "allowed",
  "default_profile",
  "locked",
]

/** The text of a policy saved without its source text. */
export const policyText = (policy: NetworkPolicy) =>
  formatJsonc(policy, KEY_ORDER)

/**
 * The template the controller gives a new network: every node may connect
 * to every other, and every node may reach every other (no internet).
 */
export const TEMPLATE_POLICY: NetworkPolicy = {
  connections: [{ id: "default", a: "*", b: "*", action: "allow" }],
  grants: [{ id: "members", src: ["*"], dst: ["*"], bidirectional: false }],
  mesh: [],
  public_rules: [],
  profiles: [],
  assignments: [],
}

/** A rule ID not yet used in the first three layers: `<stem>-<n>`. */
export function freshId(policy: NetworkPolicy, stem: string): string {
  const used = new Set(
    SHARED_ID_SECTIONS.flatMap((section) =>
      policy[section].map((rule) => rule.id)
    )
  )
  let n = 1
  while (used.has(`${stem}-${n}`)) n += 1
  return `${stem}-${n}`
}

// --- Controller diagnostics -------------------------------------------------

/**
 * A path segment pinned to a rule's ID instead of its position, so a
 * diagnostic on the saved policy still finds its rule after the draft has
 * reordered or added rules.
 */
export type Anchor = string | number | { id: string }

export type Placed =
  /** On a rule or a field of the document. */
  | { kind: "field"; anchor: Anchor[] }
  /** A planned path the controller could not find from this node. */
  | { kind: "path"; nodeId: string }
  /** About the whole policy, or a field the document does not hold. */
  | { kind: "general" }

export type PlacedDiagnostic = Diagnostic & { placed: Placed }

const NODE_HEX = /^[0-9a-f]{64}$/

/** `grants[1].dst[0]`, `mesh.<rule id>`, `automatic.regions[CN]` as path segments. */
export function fieldPath(field: string): (string | number)[] {
  const segments: (string | number)[] = []
  const pattern = /([^.[\]]+)|\[([^\]]*)\]/g
  for (const match of field.matchAll(pattern)) {
    if (match[1] !== undefined) segments.push(match[1])
    else if (match[2] !== undefined)
      segments.push(/^[0-9]+$/.test(match[2]) ? Number(match[2]) : match[2])
  }
  return segments
}

const hasId = (value: unknown): value is { id: string } =>
  typeof value === "object" &&
  value !== null &&
  "id" in value &&
  typeof value.id === "string"

/** Places one of the controller's diagnostics on the policy it was computed for. */
export function placeDiagnostic(
  diagnostic: Diagnostic,
  saved: NetworkPolicy | null
): PlacedDiagnostic {
  const path = fieldPath(diagnostic.field)
  const general = { ...diagnostic, placed: { kind: "general" as const } }
  if (!saved || path.length === 0 || typeof path[0] !== "string") return general
  // `mesh.<x>`: a rule's lint by its ID, or a path failure from a node.
  if (path[0] === "mesh" && typeof path[1] === "string") {
    const id = path[1]
    if (saved.mesh.some((rule) => rule.id === id))
      return {
        ...diagnostic,
        placed: { kind: "field", anchor: ["mesh", { id }] },
      }
    if (NODE_HEX.test(id))
      return { ...diagnostic, placed: { kind: "path", nodeId: id } }
    return general
  }
  if (!(path[0] in saved)) return general
  const anchor: Anchor[] = []
  let value: unknown = saved
  for (const segment of path) {
    const next: unknown =
      typeof segment === "number"
        ? Array.isArray(value)
          ? value[segment]
          : undefined
        : typeof value === "object" && value !== null && segment in value
          ? (value as Record<string, unknown>)[segment]
          : undefined
    if (next === undefined) break
    anchor.push(
      typeof segment === "number" && hasId(next) ? { id: next.id } : segment
    )
    value = next
  }
  return { ...diagnostic, placed: { kind: "field", anchor } }
}

/** The anchor's path in another version of the policy; null when its rule is gone. */
export function locate(
  policy: unknown,
  anchor: readonly Anchor[]
): JsonPath | null {
  const path: (string | number)[] = []
  let value: unknown = policy
  for (const segment of anchor) {
    if (typeof segment === "object") {
      if (!Array.isArray(value)) return null
      const index = value.findIndex(
        (item) => hasId(item) && item.id === segment.id
      )
      if (index < 0) return null
      path.push(index)
      value = value[index]
    } else {
      path.push(segment)
      value =
        typeof value === "object" && value !== null
          ? (value as Record<string | number, unknown>)[segment]
          : undefined
    }
  }
  return path
}

/** The anchor names this rule: `[section, {id}]` or within it. */
export function onRule(
  anchor: readonly Anchor[],
  section: string,
  id: string,
  ...within: Anchor[]
): boolean {
  const head = [section, { id }, ...within]
  return head.every((segment, index) => {
    const at = anchor[index]
    if (typeof segment === "object")
      return typeof at === "object" && at.id === segment.id
    return at === segment
  })
}
