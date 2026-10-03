import { useEffect, useId, useLayoutEffect, useMemo, useRef } from "react"
import { create } from "zustand"

import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { useKeptWhile } from "@/lib/use-kept"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { COMMIT_BOTTOM, COMMIT_SHEET_INSET } from "./layout"
import type { PageId } from "./pages"
import { Surface } from "./surface"

/** What the editor on screen offers to save: one draft, against the revision it was loaded at. */
type Commit = {
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
  // The actions close over the editor's latest render; the bar calls
  // through to them, so the store changes only when what it shows does.
  const latest = useRef(commit)
  useLayoutEffect(() => {
    latest.current = commit
  })
  const page = commit?.page ?? null
  const problem = commit?.problem ?? null
  const outdated = commit?.outdated ?? false
  const saving = commit?.saving ?? false
  const offered: Commit | null = useMemo(
    () =>
      page === null
        ? null
        : {
            page,
            problem,
            outdated,
            saving,
            save: () => latest.current?.save(),
            discard: () => latest.current?.discard(),
            overwrite: () => latest.current?.overwrite(),
          },
    [page, problem, outdated, saving]
  )
  useEffect(() => {
    if (offered) useCommitStore.setState({ owner, commit: offered })
    else if (useCommitStore.getState().owner === owner)
      useCommitStore.setState({ owner: null, commit: null })
  }, [owner, offered])
  useEffect(
    () => () => {
      if (useCommitStore.getState().owner === owner)
        useCommitStore.setState({ owner: null, commit: null })
    },
    [owner]
  )
}

/**
 * The bar inside a page's workspace frame (drawn by `PageFrame`): it rises
 * at the bottom of the workspace while that workspace holds a draft, and
 * leaves with it. Being inside the frame, popups opened from the editor
 * float above it. Its size is fixed, so its states only change words.
 * ⌘S / Ctrl+S saves.
 */
export function CommitBar({
  page,
  workspaceShown,
  mobile,
}: {
  page: PageId
  workspaceShown: boolean
  mobile: boolean
}) {
  const commit = useCommitStore((store) => store.commit)
  const shown = workspaceShown && commit !== null && commit.page === page
  const current = useKeptWhile(shown, commit)

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

  if (!current || current.page !== page) return null
  const busy = current.saving
  return (
    <div
      className="pointer-events-none absolute flex justify-center"
      style={
        mobile
          ? {
              left: COMMIT_SHEET_INSET,
              right: COMMIT_SHEET_INSET,
              bottom: COMMIT_SHEET_INSET,
            }
          : { left: 0, right: 0, bottom: COMMIT_BOTTOM }
      }
    >
      <Surface
        shown={shown}
        from="bottom"
        role="status"
        aria-label={m.commit_unsaved()}
        className="glass pointer-events-auto flex h-(--bar-height) w-full max-w-105 items-center gap-1.5 rounded-full pr-1.5 pl-5"
      >
        {/* Outdated (the controller moved on) is a warning; a field that will not save is an error. */}
        <span
          className={cn(
            "min-w-0 flex-1 truncate text-row",
            current.outdated
              ? "text-warning"
              : current.problem
                ? "text-destructive"
                : undefined
          )}
        >
          {current.outdated
            ? m.commit_outdated()
            : (current.problem ?? m.commit_unsaved())}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="h-8 min-w-16 rounded-full px-3.5"
          disabled={busy}
          onClick={current.discard}
        >
          {m.commit_discard()}
        </Button>
        <Button
          size="sm"
          // Wide enough for its longest label with the spinner: a state change only changes words.
          className="h-8 min-w-28 rounded-full px-3.5"
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
}
