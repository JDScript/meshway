import type { ConfigurationChange } from "@meshway/control-client"

import { pointerSegments } from "./jsonc"

// The controller's change list (JSON Pointer and kind, one per effective
// change), grouped by the configuration's top-level sections for reading,
// and the values a change names in two non-secret configuration views.

/** The configuration's sections, in reading order. */
export const SECTIONS = [
  "access",
  "topology",
  "routing",
  "groups",
  "tagsOwners",
  "dns",
  "addressPools",
  "nodes",
  "ruleSets",
  "defaults",
] as const

export type SectionKey = (typeof SECTIONS)[number]

export const isSection = (value: string): value is SectionKey =>
  SECTIONS.some((section) => section === value)

/** The top-level key a pointer is under; "" for the whole document. */
export function sectionOf(pointer: string): string {
  return pointerSegments(pointer)?.[0] ?? ""
}

/** What the controller binds beside the configuration (editing.md): changes under `/bindings`, which no configuration view holds. */
export const BINDINGS = ["users", "nodeUsers", "listeners", "ruleSets"] as const

export type BindingKey = (typeof BINDINGS)[number]

/** The binding a change is to; "other" for one the console does not know, null outside `/bindings`. */
export function bindingOf(pointer: string): BindingKey | "other" | null {
  const segments = pointerSegments(pointer)
  if (segments?.[0] !== "bindings") return null
  return BINDINGS.find((key) => key === segments[1]) ?? "other"
}

export type ChangeGroup = {
  section: string
  changes: ConfigurationChange[]
}

/**
 * The changes by section, sections in reading order (any the console does
 * not know after them, by name), changes in the controller's order.
 */
export function groupChanges(
  changes: readonly ConfigurationChange[]
): ChangeGroup[] {
  const groups = new Map<string, ConfigurationChange[]>()
  for (const change of changes) {
    const section = sectionOf(change.path)
    const list = groups.get(section)
    if (list) list.push(change)
    else groups.set(section, [change])
  }
  const rank = (section: string) => {
    const at = SECTIONS.findIndex((known) => known === section)
    return at < 0 ? SECTIONS.length : at
  }
  return [...groups.entries()]
    .sort(([a], [b]) => rank(a) - rank(b) || (a < b ? -1 : a > b ? 1 : 0))
    .map(([section, list]) => ({ section, changes: list }))
}

export type Found = { found: true; value: unknown } | { found: false }

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)

/** The value at `pointer` in `document`, when it holds one. */
export function valueAt(document: unknown, pointer: string): Found {
  const segments = pointerSegments(pointer)
  if (segments === null) return { found: false }
  let value: unknown = document
  for (const segment of segments) {
    if (Array.isArray(value)) {
      if (!/^(0|[1-9][0-9]*)$/.test(segment)) return { found: false }
      const index = Number(segment)
      if (index >= value.length) return { found: false }
      value = value[index]
    } else if (isRecord(value)) {
      if (!Object.hasOwn(value, segment)) return { found: false }
      value = value[segment]
    } else return { found: false }
  }
  return { found: true, value }
}

/** How many entries a section holds, where it is a list or a map; null where it is neither. */
export function entryCount(value: unknown): number | null {
  if (Array.isArray(value)) return value.length
  if (typeof value === "object" && value !== null)
    return Object.keys(value).length
  return null
}
