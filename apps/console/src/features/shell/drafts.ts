import { create } from "zustand"

import { onSessionChange } from "@/lib/session"

import type { PageId } from "./pages"

// Which pages of which network hold unsaved edits, for the dock's marks and
// the warning before the tab closes. The drafts themselves live with their
// editors (`features/configuration/draft.ts`), in this tab's memory only,
// per sign-in: when it starts or ends, every draft drops.

type Index = Record<string, Partial<Record<PageId, true>>>

const useIndex = create<Index>(() => ({}))
const resets = new Set<() => void>()

/** Says whether `page` of `networkId` holds a draft. */
export function markDraft(networkId: string, page: PageId, held: boolean) {
  const pages = useIndex.getState()[networkId] ?? {}
  if ((pages[page] === true) === held) return
  const next = { ...pages }
  if (held) next[page] = true
  else delete next[page]
  useIndex.setState({ [networkId]: next })
}

/** Registers how a kind of draft drops all it holds; called on every sign-in change. */
export function onDropDrafts(reset: () => void) {
  resets.add(reset)
}

onSessionChange(() => {
  for (const reset of resets) reset()
  useIndex.setState({}, true)
})

const NONE: Partial<Record<PageId, true>> = {}

/** The pages of `networkId` holding a draft. */
export function useDraftPages(networkId: string) {
  return useIndex((index) => index[networkId] ?? NONE)
}

window.addEventListener("beforeunload", (event) => {
  if (
    Object.values(useIndex.getState()).some(
      (pages) => Object.keys(pages).length > 0
    )
  )
    event.preventDefault()
})
