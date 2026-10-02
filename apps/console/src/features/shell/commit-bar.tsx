import { useEffect, useId, useState } from "react"
import { createPortal } from "react-dom"
import { create } from "zustand"

import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { useIsMobile } from "@/hooks/use-mobile"
import { m } from "@/paraglide/messages.js"

import { CHROME } from "./glass-drawer"
import {
  EDGE,
  MOBILE_EDGE,
  MOBILE_SHEET_BOTTOM,
  PAGE_LEFT,
  WORKSPACE_LEFT,
} from "./layout"
import { PAGES, useCurrentPage, useWorkspaceOpen, type PageId } from "./pages"
import { Surface } from "./surface"

/** What the editor on screen offers to save: one draft, against the revision it was loaded at. */
export type Commit = {
  page: PageId
  /** What stops saving (the first invalid field); null when it can be saved. */
  problem: string | null
  /** The controller moved past the draft's revision. */
  outdated: boolean
  saving: boolean
  save: () => void
  discard: () => void
  /** Saves the draft over the controller's newer revision. */
  overwrite: () => void
}

const SHEET_INSET = 6

const useCommitStore = create<{ owner: string | null; commit: Commit | null }>(
  () => ({ owner: null, commit: null })
)

/**
 * Offers the open editor's draft to the commit bar; null while there is no
 * draft. One editor is on screen at a time (one workspace), so the last one
 * to offer owns the bar until it unmounts.
 */
export function useCommit(commit: Commit | null) {
  const owner = useId()
  useEffect(() => {
    if (commit) useCommitStore.setState({ owner, commit })
    else if (useCommitStore.getState().owner === owner)
      useCommitStore.setState({ owner: null, commit: null })
  })
  useEffect(
    () => () => {
      if (useCommitStore.getState().owner === owner)
        useCommitStore.setState({ owner: null, commit: null })
    },
    [owner]
  )
}

/**
 * One floating bar for every editor: it rises at the bottom of the open
 * workspace while that workspace holds a draft. Its size is fixed, so its
 * states only change words. ⌘S / Ctrl+S saves.
 */
export function CommitBar() {
  const commit = useCommitStore((store) => store.commit)
  const page = useCurrentPage()
  const workspaceOpen = useWorkspaceOpen()
  const mobile = useIsMobile()
  const shown = commit !== null && commit.page === page && workspaceOpen
  const [kept, setKept] = useState(commit)
  if (shown && commit !== kept) setKept(commit)
  const current = shown ? commit : kept

  useEffect(() => {
    if (!shown) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "s" || !(event.metaKey || event.ctrlKey)) return
      event.preventDefault()
      if (!commit.saving && !commit.outdated && commit.problem === null)
        commit.save()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [shown, commit])

  if (!current) return null
  const busy = current.saving
  const bar = (
    // Chrome: on phones, touching the bar must not dismiss the sheet it serves.
    <div
      {...CHROME}
      className="pointer-events-none fixed z-60 flex justify-center"
      style={
        mobile
          ? // Inside the sheet, concentric with its corners: 28 − 6 = 22, the bar's own radius.
            {
              left: MOBILE_EDGE + SHEET_INSET,
              right: MOBILE_EDGE + SHEET_INSET,
              bottom: MOBILE_SHEET_BOTTOM + SHEET_INSET,
            }
          : {
              left: PAGES[current.page].panel ? WORKSPACE_LEFT : PAGE_LEFT,
              right: EDGE,
              bottom: EDGE + 16,
            }
      }
    >
      <Surface
        shown={shown}
        from="bottom"
        role="status"
        aria-label={m.commit_label()}
        className="glass pointer-events-auto flex h-(--bar-height) w-full max-w-105 items-center gap-1.5 rounded-full pr-1.5 pl-5"
      >
        <span
          className={
            current.problem && !current.outdated
              ? "min-w-0 flex-1 truncate text-row text-warning"
              : "min-w-0 flex-1 truncate text-row"
          }
        >
          {current.outdated
            ? m.commit_outdated()
            : (current.problem ?? m.commit_unsaved())}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="h-8 rounded-full px-3.5"
          disabled={busy}
          onClick={current.discard}
        >
          {m.commit_discard()}
        </Button>
        <Button
          size="sm"
          className="h-8 rounded-full px-3.5"
          disabled={busy || current.problem !== null}
          aria-busy={busy}
          onClick={current.outdated ? current.overwrite : current.save}
        >
          {busy ? <Spinner /> : null}
          {current.outdated ? m.commit_overwrite() : m.commit_save()}
        </Button>
      </Surface>
    </div>
  )
  return createPortal(bar, document.body)
}
