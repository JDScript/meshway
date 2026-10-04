import type {
  ConfigurationIssue,
  ConfigurationOperation,
  ConfigurationView,
  PreparationRequirement,
} from "@meshway/control-client"

import { withValue, type FormPart, type FormValues } from "./operations"

// One configuration draft: what this sign-in is editing in one network,
// against the revision it was loaded at. These are its transitions, kept
// pure so they can be tested without React; `draft.ts` holds the entries.
//
// A draft holds one kind of input, never both: the JSONC source exactly as
// typed (invalid text and comments stay, and it is never rebuilt from parsed
// data), or the typed operations the section forms made, over the
// non-secret view of the same revision. One never turns into the other: a
// form refuses while the source holds edits, and the source while the forms
// do. A newer revision never moves a draft with edits (strict CAS: applying
// it says the controller moved on, and the person decides).

export type DraftInput =
  | { kind: "document"; text: string }
  | {
      kind: "operations"
      /** The view the forms started from, at the draft's base revision. */
      base: ConfigurationView
      /** Never empty: a draft without operations is no operations draft. */
      ops: readonly ConfigurationOperation[]
    }

/** What the controller said about an input when it last checked it. */
export type Checked = {
  input: DraftInput
  issues: readonly ConfigurationIssue[]
  requirements: readonly PreparationRequirement[]
}

export type DraftEntry = {
  /** The revision the draft was read at: it is applied against it. */
  baseRevision: number
  /** The source as the controller holds it at that revision, once read. Secret: memory only. */
  baseText: string | null
  input: DraftInput
  checked: Checked | null
}

/** The draft holds edits of its own. */
export function isDirty(entry: DraftEntry | null | undefined): boolean {
  if (!entry) return false
  return entry.input.kind === "operations"
    ? entry.input.ops.length > 0
    : entry.input.text !== entry.baseText
}

/** The draft holds source edits: the forms must not edit over them. */
export const holdsDocument = (entry: DraftEntry | null | undefined): boolean =>
  isDirty(entry) && entry?.input.kind === "document"

/** The draft holds form edits: the source must not be edited over them. */
export const holdsOperations = (
  entry: DraftEntry | null | undefined
): boolean => isDirty(entry) && entry?.input.kind === "operations"

/** The operations a draft holds; none for a document. */
export const operationsOf = (
  entry: DraftEntry | null | undefined
): readonly ConfigurationOperation[] =>
  entry?.input.kind === "operations" ? entry.input.ops : []

/**
 * The source read from the controller: the draft over it when there was
 * none, or none with edits at another revision. A draft with edits stays
 * as it is, whatever was read.
 */
export function loadSource(
  entry: DraftEntry | null,
  source: { revision: number; text: string }
): DraftEntry {
  if (entry && isDirty(entry)) return entry
  if (
    entry &&
    entry.baseRevision === source.revision &&
    entry.baseText !== null
  )
    return entry
  return {
    baseRevision: source.revision,
    baseText: source.text,
    input: { kind: "document", text: source.text },
    checked: null,
  }
}

/** Typed text. Refused while the forms hold edits. */
export function editText(entry: DraftEntry, text: string): DraftEntry {
  if (entry.input.kind !== "document") return entry
  if (entry.input.text === text) return entry
  return { ...entry, input: { kind: "document", text } }
}

/**
 * A form set `part` to `value`, over `view` (the current configuration
 * when the draft has no edits; the draft's own base when it does). Refused
 * while the source holds edits. Setting a part back to its base value
 * drops its operation; the last one dropped ends the draft's edits.
 */
export function editPart<P extends FormPart>(
  entry: DraftEntry | null,
  view: { revision: number; configuration: ConfigurationView },
  part: P,
  value: FormValues[P]
): DraftEntry | null {
  return editOperations(entry, view, (ops, base) =>
    withValue(ops, base, part, value)
  )
}

/**
 * The draft's operations changed by `change`, over the same base as
 * `editPart`: how the node workspace's edits (`node-operations.ts`) enter
 * the draft. Refused while the source holds edits.
 */
export function editOperations(
  entry: DraftEntry | null,
  view: { revision: number; configuration: ConfigurationView },
  change: (
    ops: readonly ConfigurationOperation[],
    base: ConfigurationView
  ) => ConfigurationOperation[]
): DraftEntry | null {
  if (holdsDocument(entry)) return entry
  const held = entry?.input.kind === "operations" ? entry.input : null
  const base = held ? held.base : view.configuration
  const baseRevision = held && entry ? entry.baseRevision : view.revision
  const ops = change(held?.ops ?? [], base)
  // The source read at the same revision is still the base.
  const baseText =
    entry && entry.baseRevision === baseRevision ? entry.baseText : null
  if (ops.length === 0)
    return baseText === null
      ? null
      : {
          baseRevision,
          baseText,
          input: { kind: "document", text: baseText },
          checked: null,
        }
  return {
    baseRevision,
    baseText,
    input: { kind: "operations", base, ops },
    checked: entry?.checked ?? null,
  }
}

/** Back to the configuration as it was read. */
export function discardEdits(entry: DraftEntry): DraftEntry | null {
  if (entry.baseText === null) return null
  return {
    ...entry,
    input: { kind: "document", text: entry.baseText },
    checked: null,
  }
}

export function withChecked(entry: DraftEntry, checked: Checked): DraftEntry {
  return { ...entry, checked }
}

/** Two inputs ask the controller the same thing. */
function sameInput(a: DraftInput, b: DraftInput): boolean {
  if (a.kind === "document") return b.kind === "document" && a.text === b.text
  if (b.kind === "document") return false
  return JSON.stringify(a.ops) === JSON.stringify(b.ops)
}

/** What the controller said is about the draft as it is now. */
export const checkedNow = (entry: DraftEntry | null): Checked | null =>
  entry?.checked && sameInput(entry.checked.input, entry.input)
    ? entry.checked
    : null

/**
 * After the controller applied `input`: the draft is done when it still
 * holds that input (the configuration is then read again at the new revision).
 */
export function afterApply(
  entry: DraftEntry | null,
  input: DraftInput
): DraftEntry | null {
  if (!entry) return null
  return sameInput(entry.input, input) ? null : entry
}

/** The source must be read (again): none read yet, or a draft without edits behind the controller. */
export function needsSource(
  entry: DraftEntry | null,
  currentRevision: number | undefined
): boolean {
  if (!entry) return true
  if (isDirty(entry)) return false
  if (entry.baseText === null) return true
  return currentRevision !== undefined && currentRevision !== entry.baseRevision
}

/** The controller has a revision other than the draft's base: applying it would conflict. */
export function isOutdated(
  entry: DraftEntry | null,
  currentRevision: number | undefined
): boolean {
  return (
    entry !== null &&
    isDirty(entry) &&
    currentRevision !== undefined &&
    currentRevision !== entry.baseRevision
  )
}
