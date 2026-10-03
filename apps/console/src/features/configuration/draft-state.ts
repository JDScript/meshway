import type {
  ConfigurationIssue,
  PreparationRequirement,
} from "@meshway/control-client"

// One configuration draft: what this sign-in is editing in one network,
// against the revision it was loaded at. These are its transitions, kept
// pure so they can be tested without React; `draft.ts` holds the entries.
//
// The draft's text is the JSONC source exactly as typed: invalid text and
// comments stay, and it is never rebuilt from parsed data. A newer revision
// never moves a draft with edits (strict CAS: applying it says the
// controller moved on, and the person decides). Typed operations from the
// section forms will be a second kind of input; only the document exists now.

export type DraftInput = { kind: "document"; text: string }

/** What the controller said about a text when it last checked it. */
export type Checked = {
  text: string
  issues: readonly ConfigurationIssue[]
  requirements: readonly PreparationRequirement[]
}

export type DraftEntry = {
  /** The revision the source was read at: the draft is applied against it. */
  baseRevision: number
  /** The source as the controller holds it at that revision. Secret: memory only. */
  baseText: string
  input: DraftInput
  checked: Checked | null
}

/** The draft holds edits of its own. */
export const isDirty = (entry: DraftEntry | null | undefined): boolean =>
  entry !== null &&
  entry !== undefined &&
  entry.input.kind === "document" &&
  entry.input.text !== entry.baseText

/** The draft's text (the source itself while there are no edits). */
export const draftText = (entry: DraftEntry): string => entry.input.text

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
  if (entry && entry.baseRevision === source.revision) return entry
  return {
    baseRevision: source.revision,
    baseText: source.text,
    input: { kind: "document", text: source.text },
    checked: null,
  }
}

/** Typed text. What the controller said about earlier text stays, marked by its own text. */
export function editText(entry: DraftEntry, text: string): DraftEntry {
  if (entry.input.text === text) return entry
  return { ...entry, input: { kind: "document", text } }
}

/** Back to the source as it was read. */
export function discardEdits(entry: DraftEntry): DraftEntry {
  return {
    ...entry,
    input: { kind: "document", text: entry.baseText },
    checked: null,
  }
}

export function withChecked(entry: DraftEntry, checked: Checked): DraftEntry {
  return { ...entry, checked }
}

/**
 * After the controller applied `text`: the draft is done when it still
 * holds that text (the source is then read again at the new revision).
 */
export function afterApply(
  entry: DraftEntry | null,
  text: string
): DraftEntry | null {
  if (!entry) return null
  return entry.input.text === text ? null : entry
}

/** The source must be read (again): no draft yet, or one without edits behind the controller. */
export function needsSource(
  entry: DraftEntry | null,
  currentRevision: number | undefined
): boolean {
  if (!entry) return true
  if (isDirty(entry)) return false
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
