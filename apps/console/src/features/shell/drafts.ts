import { useCallback, useEffect, useMemo } from "react"
import { create, type StoreApi, type UseBoundStore } from "zustand"

import { sameData } from "@/lib/same-data"

import type { PageId } from "./pages"

// Unsaved edits, in this tab's memory only (never in storage). A draft is
// one editable resource of one network, with the revision it was loaded at:
// refreshes never move it, and it is saved against that revision (CAS). It
// survives page changes, network switches and Back; signing out drops every
// draft (a sign-in changed in another tab reloads this one). Closing or
// reloading the tab with a draft asks first.

/** What the controller holds: the editable value and its revision. */
export type Saved<T> = { value: T; revision: number }

type Entry<T> = { base: T; baseRevision: number; value: T }

/** Which pages of which network hold a draft: the dock marks them. */
const useIndex = create<Record<string, Partial<Record<PageId, number>>>>(
  () => ({})
)
const resets: (() => void)[] = []

/** Drops every draft (signing out). */
export function dropDrafts() {
  for (const reset of resets) reset()
  useIndex.setState({}, true)
}

export function useDraftPages(
  networkId: string
): Partial<Record<PageId, number>> {
  return useIndex((index) => index[networkId] ?? NONE)
}
const NONE: Partial<Record<PageId, number>> = {}

window.addEventListener("beforeunload", (event) => {
  if (
    Object.values(useIndex.getState()).some((pages) =>
      Object.values(pages).some((count) => (count ?? 0) > 0)
    )
  ) {
    event.preventDefault()
  }
})

export type Draft<T> = {
  /** The draft, or the saved value when there is none; undefined until the first read. */
  value: T | undefined
  dirty: boolean
  /** The controller has moved past the revision the draft started from: saving would conflict. */
  outdated: boolean
  /** The revision to save against. */
  baseRevision: number | undefined
  edit: (update: (current: T) => T) => void
  discard: () => void
  /** Keeps the edits but takes the controller's current revision as the base (overwriting what changed there). */
  rebase: () => void
}

/**
 * One kind of draft (a network's settings, its policy), created once per
 * resource at module level. The hook takes the controller's current value
 * and returns the draft over it; an edit that returns to the saved value is
 * no draft.
 */
export function createDraft<T>(
  page: PageId
): (networkId: string, saved: Saved<T> | undefined) => Draft<T> {
  const store: UseBoundStore<StoreApi<Record<string, Entry<T>>>> = create<
    Record<string, Entry<T>>
  >(() => ({}))
  resets.push(() => store.setState({}, true))

  const write = (networkId: string, entry: Entry<T> | null) => {
    store.setState((entries) => {
      const next = { ...entries }
      if (entry) next[networkId] = entry
      else delete next[networkId]
      return next
    }, true)
    useIndex.setState((index) => {
      const pages = { ...index[networkId] }
      const count = (pages[page] ?? 0) + (entry ? 1 : -1)
      if (count > 0) pages[page] = count
      else delete pages[page]
      return { ...index, [networkId]: pages }
    })
  }

  return function useDraft(networkId, saved) {
    const entry = store((entries) => entries[networkId])

    const edit = useCallback(
      (update: (current: T) => T) => {
        const current = store.getState()[networkId]
        const base = current
          ? { value: current.base, revision: current.baseRevision }
          : saved
        if (!base) return
        const value = update(current ? current.value : base.value)
        const had = current !== undefined
        if (sameData(value, base.value)) {
          if (had) write(networkId, null)
        } else if (had)
          store.setState((entries) => ({
            ...entries,
            [networkId]: { ...current, value },
          }))
        else
          write(networkId, {
            base: base.value,
            baseRevision: base.revision,
            value,
          })
      },
      [networkId, saved]
    )
    const discard = useCallback(() => {
      if (store.getState()[networkId]) write(networkId, null)
    }, [networkId])
    const rebase = useCallback(() => {
      const current = store.getState()[networkId]
      if (!current || !saved) return
      if (sameData(current.value, saved.value)) write(networkId, null)
      else
        store.setState((entries) => ({
          ...entries,
          [networkId]: {
            base: saved.value,
            baseRevision: saved.revision,
            value: current.value,
          },
        }))
    }, [networkId, saved])

    // A draft that the controller now holds as saved (saved here, or the same edit elsewhere) is done.
    useEffect(() => {
      if (entry && saved && sameData(entry.value, saved.value))
        write(networkId, null)
    }, [entry, saved, networkId])

    return useMemo(
      () => ({
        value: entry ? entry.value : saved?.value,
        dirty: entry !== undefined,
        outdated:
          entry !== undefined &&
          saved !== undefined &&
          saved.revision !== entry.baseRevision,
        baseRevision: entry ? entry.baseRevision : saved?.revision,
        edit,
        discard,
        rebase,
      }),
      [entry, saved, edit, discard, rebase]
    )
  }
}
