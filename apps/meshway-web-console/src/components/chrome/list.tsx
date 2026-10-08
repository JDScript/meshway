import { cn } from "@meshway/ui/cn"
import {
  useRef,
  type ComponentProps,
  type KeyboardEvent,
  type ReactNode,
} from "react"

import { Skeleton } from "@/components/ui/skeleton"

// The rows of a page panel's list (nodes, tags, members, invitations,
// settings sections). Every row has the same shape: a 16px lead slot (a
// dot, an avatar or a glyph), a title over one line of detail, one trailing
// column; so the text starts at the same x on every page. Rows sit 2px
// apart, so a hovered row never merges with the selected one.

/** The scrolling list under a panel's header. */
export function PanelList({
  className,
  ...props
}: ComponentProps<"ul"> & { "aria-labelledby": string }) {
  return (
    <ul
      className={cn(
        "scroll-edge flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-2",
        className
      )}
      {...props}
    />
  )
}

const ROW = "h-14 gap-3 px-3"
const LEAD = "flex size-4 shrink-0 items-center justify-center"

function RowBody({
  lead,
  title,
  subtitle,
  trail,
}: {
  lead: ReactNode
  title: ReactNode
  subtitle?: ReactNode
  trail?: ReactNode
}) {
  return (
    <>
      <span className={LEAD}>{lead}</span>
      <span className="min-w-0 flex-1">
        <span className="flex min-w-0 items-center gap-1.5 text-row font-medium">
          {title}
        </span>
        {subtitle !== undefined ? (
          <span className="flex h-4 min-w-0 items-center gap-1.5 text-meta text-muted-foreground">
            {subtitle}
          </span>
        ) : null}
      </span>
      {trail !== undefined ? (
        <span className="flex shrink-0 items-center text-meta text-muted-foreground tabular-nums">
          {trail}
        </span>
      ) : null}
    </>
  )
}

type RowContent = {
  lead: ReactNode
  /** Truncate long parts yourself (`truncate`): the title line also holds marks. */
  title: ReactNode
  subtitle?: ReactNode
  trail?: ReactNode
}

/** A choosable row: opens what it names; `current` marks the open one. */
export function ListRow({
  lead,
  title,
  subtitle,
  trail,
  current = false,
  rowId,
  className,
  ...props
}: RowContent & {
  current?: boolean
  /** Identifies the row for the arrow keys (`useListKeys`). */
  rowId?: string
} & Omit<ComponentProps<"button">, "title" | "type">) {
  return (
    <li>
      <button
        type="button"
        data-row-id={rowId}
        aria-current={current ? "true" : undefined}
        className={cn("list-row group/row", ROW, className)}
        {...props}
      >
        <RowBody lead={lead} title={title} subtitle={subtitle} trail={trail} />
      </button>
    </li>
  )
}

/** A row that is not chosen (an invitation): the same shape; its trailing column may hold a control. */
export function StaticRow({ lead, title, subtitle, trail }: RowContent) {
  return (
    <li className={cn("flex items-center rounded-row", ROW)}>
      <RowBody lead={lead} title={title} subtitle={subtitle} trail={trail} />
    </li>
  )
}

/** Rows standing in for a list that is being read. */
export function ListPlaceholders({ rows = 4 }: { rows?: number }) {
  return Array.from({ length: rows }, (_, index) => (
    <li key={index} className={cn("flex items-center", ROW)}>
      <span className={LEAD}>
        <Skeleton className="size-2 rounded-full" />
      </span>
      <Skeleton className="h-3 w-32 rounded-full" />
    </li>
  ))
}

/** The one line of a list with no rows: empty, nothing matching, or not read. */
export function ListNote({ children }: { children: ReactNode }) {
  return (
    <li className={cn("flex items-center text-row text-muted-foreground", ROW)}>
      <span className={LEAD} />
      {children}
    </li>
  )
}

/**
 * The arrow keys walk a list's rows (by `rowId`) and, while one is open,
 * carry it along. Returns the list's ref and its key handler.
 */
export function useListKeys({
  ids,
  openId,
  onOpen,
}: {
  ids: readonly string[]
  openId: string | null
  onOpen: (id: string) => void
}) {
  const ref = useRef<HTMLUListElement>(null)
  const onKeyDown = (event: KeyboardEvent<HTMLUListElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return
    event.preventDefault()
    const focused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement.dataset.rowId
        : undefined
    const index = ids.indexOf(focused ?? openId ?? "")
    const next =
      ids[
        index === -1
          ? 0
          : Math.min(
              ids.length - 1,
              Math.max(0, index + (event.key === "ArrowDown" ? 1 : -1))
            )
      ]
    if (next === undefined) return
    ref.current
      ?.querySelector<HTMLButtonElement>(`[data-row-id="${CSS.escape(next)}"]`)
      ?.focus()
    if (openId) onOpen(next)
  }
  return { ref, onKeyDown }
}
