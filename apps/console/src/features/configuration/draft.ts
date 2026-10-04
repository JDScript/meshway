import type { ConfigurationView } from "@meshway/control-client"
import { useCallback, useMemo } from "react"
import { create } from "zustand"

import { markDraft, onDropDrafts } from "@/features/shell/drafts"
import { currentSession } from "@/lib/session"

import {
  afterApply,
  discardEdits,
  editPart,
  editText,
  isDirty,
  loadSource,
  withChecked,
  type Checked,
  type DraftEntry,
  type DraftInput,
} from "./draft-state"
import type { FormPart, FormValues } from "./operations"

// The configuration drafts of this tab: one per sign-in and network, kept
// across the network's pages and routes, in memory only (the source holds
// secrets: never storage, the URL or the query cache). Signing out or in
// as someone else drops them (`features/shell/drafts.ts`).

const useEntries = create<Record<string, DraftEntry>>(() => ({}))
onDropDrafts(() => useEntries.setState({}, true))

/** Network IDs are UUIDs: the first "/" ends the session ID, so keys never collide. */
const keyOf = (networkId: string) =>
  `${currentSession()?.id ?? ""}/${networkId}`

function update(
  networkId: string,
  change: (entry: DraftEntry | null) => DraftEntry | null
) {
  const key = keyOf(networkId)
  const current = useEntries.getState()[key] ?? null
  const next = change(current)
  if (next === current) return
  useEntries.setState((entries) => {
    const copy = { ...entries }
    if (next) copy[key] = next
    else delete copy[key]
    return copy
  }, true)
  markDraft(networkId, "configuration", isDirty(next))
}

/** The current entry, outside React (an apply that finished after a page change). */
export const draftEntry = (networkId: string): DraftEntry | null =>
  useEntries.getState()[keyOf(networkId)] ?? null

export function useConfigurationDraft(networkId: string) {
  const key = keyOf(networkId)
  const entry = useEntries((entries) => entries[key] ?? null)

  const load = useCallback(
    (source: { revision: number; text: string }) =>
      update(networkId, (current) => loadSource(current, source)),
    [networkId]
  )
  const edit = useCallback(
    (text: string) =>
      update(networkId, (current) =>
        current ? editText(current, text) : current
      ),
    [networkId]
  )
  const editForm = useCallback(
    <P extends FormPart>(
      view: { revision: number; configuration: ConfigurationView },
      part: P,
      value: FormValues[P]
    ) => update(networkId, (current) => editPart(current, view, part, value)),
    [networkId]
  )
  const discard = useCallback(
    () =>
      update(networkId, (current) =>
        current ? discardEdits(current) : current
      ),
    [networkId]
  )
  const checked = useCallback(
    (result: Checked) =>
      update(networkId, (current) =>
        current ? withChecked(current, result) : current
      ),
    [networkId]
  )
  const applied = useCallback(
    (input: DraftInput) =>
      update(networkId, (current) => afterApply(current, input)),
    [networkId]
  )
  /** Starts over from the controller's current configuration (read again). */
  const reset = useCallback(() => update(networkId, () => null), [networkId])

  return useMemo(
    () => ({
      entry,
      dirty: isDirty(entry),
      load,
      edit,
      editForm,
      discard,
      checked,
      applied,
      reset,
    }),
    [entry, load, edit, editForm, discard, checked, applied, reset]
  )
}
