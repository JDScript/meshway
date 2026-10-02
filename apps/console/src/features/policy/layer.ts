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

/**
 * Which rule of a layer is open, by position: it follows the rule through
 * moves and renames in the same event (no frame where it closes), and is
 * mirrored into the address by ID for links.
 */
export function useOpenRule(
  rules: readonly { id: string }[],
  initial: string | null,
  onRule: (id: string | null) => void
) {
  const [open, setOpen] = useState<number | null>(() => {
    const index =
      initial === null ? -1 : rules.findIndex((rule) => rule.id === initial)
    return index < 0 ? null : index
  })
  const at = open !== null && open < rules.length ? open : null
  return {
    open: at,
    /** A stable key: the open rule keeps one while it is renamed or moved. */
    key: (index: number) =>
      index === at
        ? "open"
        : `${rules[index]?.id ?? ""}#${rules.slice(0, index).filter((rule) => rule.id === rules[index]?.id).length}`,
    toggle(index: number) {
      const next = at === index ? null : index
      setOpen(next)
      onRule(next === null ? null : (rules[index]?.id ?? null))
    },
    moved(index: number, delta: -1 | 1) {
      if (at === index) setOpen(index + delta)
      else if (at === index + delta) setOpen(index)
    },
    removed(index: number) {
      if (at === index) {
        setOpen(null)
        onRule(null)
      } else if (at !== null && at > index) setOpen(at - 1)
    },
    added(index: number, id: string) {
      setOpen(index)
      onRule(id)
    },
    renamed(id: string) {
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
