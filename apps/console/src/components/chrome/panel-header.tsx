import type { LucideIcon } from "lucide-react"
import type { ReactNode } from "react"

import { IconAction } from "@/components/chrome/icon-action"
import type { PageId } from "@/features/shell/pages"
import { surfaceTitleId } from "@/features/shell/page-frame"

/**
 * The head of a page panel: the title and how many of what it names, one
 * action at its end, then the panel's controls (a view switch, a search).
 * One height on every page, so the list below starts where it always does.
 */
export function PanelHeader({
  page,
  title,
  count,
  action,
  children,
}: {
  page: PageId
  title: string
  /** How many of what the title names; null while unread, left out where nothing is counted. */
  count?: string | number | null
  action?: ReactNode
  children?: ReactNode
}) {
  return (
    <header className="flex flex-col gap-3 px-4 pt-4 pb-3">
      <div className="flex h-7 items-center gap-2">
        <h2
          id={surfaceTitleId(page, "panel")}
          className="text-lead font-semibold tracking-tight"
        >
          {title}
        </h2>
        {count !== undefined ? (
          <span className="text-meta text-muted-foreground tabular-nums">
            {count ?? "—"}
          </span>
        ) : null}
        <span className="flex-1" />
        {action}
      </div>
      {children}
    </header>
  )
}

/** A panel's one action: an icon whose glyph meets the content edge, named by its tooltip. */
export function PanelAction(props: {
  label: string
  icon: LucideIcon
  onClick: () => void
}) {
  return <IconAction {...props} className="-mr-2" />
}
