import type {
  ConfigurationOperation,
  ConfigurationView,
} from "@meshway/control-client"
import { useMemo } from "react"
import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import {
  afterApply,
  discardEdits,
  editOperations,
  editPart,
  editText,
  isDirty,
  loadSource,
  withChecked,
  type Checked,
  type DraftEntry,
  type DraftInput,
} from "@/lib/configuration/draft-state"
import type { FormPart, FormValues } from "@/lib/configuration/operations"

import { onSessionChange } from "./session"

// The configuration drafts of this tab: one per network, kept across the
// network's pages and routes, in memory only (the source holds secrets:
// never storage, the URL or the query cache), and dropped whenever the
// sign-in starts, ends or changes. How a draft changes is `draft-state`'s
// pure transitions; this store only holds the entries.

type Drafts = Readonly<Record<string, DraftEntry>>

const draftStore = createStore<Drafts>(() => ({}))

onSessionChange(() => draftStore.setState({}, true))

type View = { revision: number; configuration: ConfigurationView }

function update(
  networkId: string,
  change: (entry: DraftEntry | null) => DraftEntry | null
) {
  const current = draftStore.getState()[networkId] ?? null
  const next = change(current)
  if (next === current) return
  draftStore.setState((drafts) => {
    const copy = { ...drafts }
    if (next) copy[networkId] = next
    else delete copy[networkId]
    return copy
  }, true)
}

/** The network's draft now, outside React (an apply that ends after its page has gone). */
export const draftOf = (networkId: string): DraftEntry | null =>
  draftStore.getState()[networkId] ?? null

/** Some network holds edits not applied yet: closing the tab asks first. */
export const anyDraftHeld = (): boolean =>
  Object.values(draftStore.getState()).some(isDirty)

/** The changes a draft takes, for one network. */
export function draftActions(networkId: string) {
  return {
    load: (source: { revision: number; text: string }) =>
      update(networkId, (current) => loadSource(current, source)),
    edit: (text: string) =>
      update(networkId, (current) =>
        current ? editText(current, text) : current
      ),
    editForm: <P extends FormPart>(view: View, part: P, value: FormValues[P]) =>
      update(networkId, (current) => editPart(current, view, part, value)),
    /** The node workspace's edits: `change` maps the operations over the draft's base. */
    editOps: (
      view: View,
      change: (
        ops: readonly ConfigurationOperation[],
        base: ConfigurationView
      ) => ConfigurationOperation[]
    ) => update(networkId, (current) => editOperations(current, view, change)),
    discard: () =>
      update(networkId, (current) =>
        current ? discardEdits(current) : current
      ),
    checked: (result: Checked) =>
      update(networkId, (current) =>
        current ? withChecked(current, result) : current
      ),
    applied: (input: DraftInput) =>
      update(networkId, (current) => afterApply(current, input)),
    /** Starts over from the controller's current configuration (read again). */
    reset: () => update(networkId, () => null),
  }
}

/** The network's draft and its changes. */
export function useConfigurationDraft(networkId: string) {
  const entry = useStore(draftStore, (drafts) => drafts[networkId] ?? null)
  const actions = useMemo(() => draftActions(networkId), [networkId])
  return useMemo(
    () => ({ entry, dirty: isDirty(entry), ...actions }),
    [entry, actions]
  )
}

/** Every network's draft (the debug panel says what each holds, never its text). */
export const useDrafts = (): Drafts => useStore(draftStore, (drafts) => drafts)

/** Whether the network's draft holds edits not applied yet (the dock's mark). */
export const useDraftHeld = (networkId: string): boolean =>
  useStore(draftStore, (drafts) => isDirty(drafts[networkId] ?? null))
