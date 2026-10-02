import type { NetworkPolicy } from "@meshway/control-client"
import { useState } from "react"

import {
  onRule,
  type Anchor,
  type PlacedDiagnostic,
  type Problem,
} from "./document"
import type { JsonPath } from "./jsonc"
import type { SelectorNames } from "./selectors"

/** What every layer view is given. */
export type LayerProps = {
  policy: NetworkPolicy
  /** The sign-in can change the policy and the view shows the draft (not a proposal). */
  editable: boolean
  names: SelectorNames
  diagnostics: readonly PlacedDiagnostic[]
  problems: readonly Problem[]
  /** The rule the address opens (by ID) when the layer is shown. */
  rule: string | null
  /** Mirrors the open rule into the address. */
  onRule: (id: string | null) => void
  edit: (update: (policy: NetworkPolicy) => NetworkPolicy) => void
  move: (path: JsonPath, index: number, delta: -1 | 1) => void
  onOpenNode: (nodeId: string) => void
}

/** The controller's notes on one rule (or on a field of it). */
export function notesOn(
  diagnostics: readonly PlacedDiagnostic[],
  section: string,
  id: string,
  ...within: Anchor[]
): PlacedDiagnostic[] {
  return diagnostics.filter(
    (diagnostic) =>
      diagnostic.placed.kind === "field" &&
      onRule(diagnostic.placed.anchor, section, id, ...within)
  )
}

/** Whether the draft's rule ID at `section[index]` is refused. */
export const idInvalid = (
  problems: readonly Problem[],
  section: string,
  index: number
) =>
  problems.some(
    (problem) =>
      problem.kind === "id" &&
      problem.path[0] === section &&
      problem.path[1] === index
  )

let serial = 0
const freshKey = () => `rule-${serial++}`

/**
 * A layer's rows: a key per row that is not its ID (so an ID edited in
 * place keeps its row, and focus, while it changes) and follows the row
 * through moves (so it glides), removals and additions; which row the
 * address points at (by ID); and which row was just added, whose first
 * selector opens at once.
 */
export function useRuleRows(
  rules: readonly { id: string }[],
  linked: string | null,
  onRule: (id: string | null) => void
) {
  const [keys, setKeys] = useState<string[]>(() => rules.map(freshKey))
  const [added, setAdded] = useState<string | null>(null)
  // Rules changed elsewhere (the source, a discard): rows are matched by position.
  let rows = keys
  if (keys.length !== rules.length) {
    rows =
      keys.length < rules.length
        ? [...keys, ...rules.slice(keys.length).map(freshKey)]
        : keys.slice(0, rules.length)
    setKeys(rows)
  }
  return {
    key: (index: number) => rows[index] ?? `rule-at-${index}`,
    current: (index: number) => linked !== null && rules[index]?.id === linked,
    autoOpen: (index: number) => added !== null && rows[index] === added,
    /** The just-added row's picker has closed. */
    settled: () => setAdded(null),
    /** Call with the edit that appends a rule. */
    added() {
      const key = freshKey()
      setKeys([...rows, key])
      setAdded(key)
    },
    removed(index: number) {
      setKeys(rows.filter((_, at) => at !== index))
      if (linked !== null && rules[index]?.id === linked) onRule(null)
    },
    moved(index: number, delta: -1 | 1) {
      const next = [...rows]
      const a = next[index]
      const b = next[index + delta]
      if (a === undefined || b === undefined) return
      next[index] = b
      next[index + delta] = a
      setKeys(next)
    },
    /** The address follows the rule it points at when that rule is renamed. */
    renamed(index: number, id: string) {
      if (linked !== null && rules[index]?.id === linked)
        onRule(id === "" ? null : id)
    },
  }
}

/** Replaces one element of an array. */
export const replaceAt = <T>(
  items: readonly T[],
  index: number,
  next: T
): T[] => items.map((item, at) => (at === index ? next : item))
