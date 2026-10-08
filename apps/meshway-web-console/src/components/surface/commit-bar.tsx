import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import { useEffect } from "react"

import { Spinner } from "@/components/ui/spinner"
import { useKeptWhile } from "@/hooks/use-kept"

import { COMMIT_BOTTOM, COMMIT_SHEET_INSET } from "./layout"
import { Surface } from "./surface"

/**
 * What a page's draft offers the bar. It states one thing: unsaved
 * (`plain`), will not save (`destructive`), or the controller moved past
 * the draft's revision (`warning`); the editor words it and its buttons.
 */
export type Commit = {
  tone: "plain" | "destructive" | "warning"
  text: string
  /** Names the bar for assistive tech. */
  label: string
  secondaryLabel: string
  primaryLabel: string
  primaryDisabled: boolean
  busy: boolean
  secondary: () => void
  primary: () => void
}

/**
 * The bar inside a page's workspace frame (drawn by `PageFrame`): it rises
 * at the bottom of the workspace while the page holds a draft, and leaves
 * with it. Its size is fixed, so its states only change words. ⌘S / Ctrl+S
 * takes the primary action.
 */
export function CommitBar({
  commit,
  workspaceShown,
  mobile,
}: {
  /** What the page's draft offers; null while there is none. */
  commit: Commit | null
  workspaceShown: boolean
  mobile: boolean
}) {
  const shown = workspaceShown && commit !== null
  const current = useKeptWhile(shown, commit)

  useEffect(() => {
    if (!workspaceShown || !commit) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "s" || !(event.metaKey || event.ctrlKey)) return
      event.preventDefault()
      if (!commit.busy && !commit.primaryDisabled) commit.primary()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [workspaceShown, commit])

  if (!current) return null
  const busy = current.busy
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
        aria-label={current.label}
        className="glass pointer-events-auto flex h-(--bar-height) w-full max-w-120 items-center gap-1.5 rounded-full pr-1.5 pl-5"
      >
        <span
          className={cn(
            "min-w-0 flex-1 truncate text-row",
            current.tone === "warning" && "text-warning",
            current.tone === "destructive" && "text-destructive"
          )}
        >
          {current.text}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="h-8 min-w-20 rounded-full px-3.5"
          disabled={busy}
          onClick={current.secondary}
        >
          {current.secondaryLabel}
        </Button>
        <Button
          size="sm"
          // Wide enough for its longest label with the spinner: a state change only changes words.
          className="h-8 min-w-28 rounded-full px-3.5"
          disabled={busy || current.primaryDisabled}
          aria-busy={busy}
          onClick={current.primary}
        >
          {busy ? <Spinner /> : null}
          {current.primaryLabel}
        </Button>
      </Surface>
    </div>
  )
}
