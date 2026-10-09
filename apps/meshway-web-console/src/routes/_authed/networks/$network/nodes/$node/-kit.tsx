import { cn } from "@meshway/ui/cn"
import { ChevronRightIcon } from "lucide-react"
import { useState, type ReactNode } from "react"

import { Skeleton } from "@/components/ui/skeleton"

// The parts every tab of a node's workspace is built from, so they read as
// one page: a titled card of rows, a summary tile, and a part shown on
// demand. Rows share one height, inset and label column.

export function Card({
  title,
  aside,
  action,
  children,
  className,
}: {
  title: string
  /** A short fact beside the title (a count). */
  aside?: ReactNode
  /** One control at the title's end. */
  action?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <section className={cn("flex min-w-0 flex-col gap-2", className)}>
      <div className="flex h-7 items-center gap-2">
        <h3 className="text-row font-medium">{title}</h3>
        {aside !== undefined ? (
          <span className="text-meta text-muted-foreground tabular-nums">
            {aside}
          </span>
        ) : null}
        <span className="flex-1" />
        {action}
      </div>
      {children}
    </section>
  )
}

/** The bordered block of rows under a card's title. */
export function Rows({ children }: { children: ReactNode }) {
  return (
    <div className="divide-y divide-divider overflow-hidden rounded-tile border border-divider">
      {children}
    </div>
  )
}

/** A row whose value is not read yet, its label in place. */
export function LoadingRow({ label }: { label: string }) {
  return (
    <Row label={label}>
      <span className="flex h-[18px] items-center">
        <Skeleton className="h-3 w-40 rounded-full" />
      </span>
    </Row>
  )
}

export function Row({
  label,
  children,
  detail,
  action,
  mono = false,
  tone,
}: {
  label: string
  children: ReactNode
  detail?: ReactNode
  action?: ReactNode
  mono?: boolean
  tone?: "muted" | "warning" | "destructive"
}) {
  return (
    <div className="grid min-h-11 grid-cols-[4.5rem_minmax(0,1fr)_auto] items-center gap-3 px-4 py-2.5 sm:grid-cols-[7rem_minmax(0,1fr)_auto]">
      <span className="truncate text-meta text-muted-foreground">{label}</span>
      <span className="min-w-0">
        <span
          className={cn(
            "block truncate text-row",
            mono && "font-mono text-[12px]",
            tone === "muted" && "text-muted-foreground",
            tone === "warning" && "text-warning",
            tone === "destructive" && "text-destructive"
          )}
        >
          {children}
        </span>
        {detail ? (
          <span className="mt-0.5 block truncate text-meta text-muted-foreground">
            {detail}
          </span>
        ) : null}
      </span>
      <span className="flex min-w-6 justify-end">{action}</span>
    </div>
  )
}

/** One number or word that sums a part up, with a line under it; choosing it opens that part. */
export function SummaryTile({
  label,
  value,
  detail,
  tone,
  loading = false,
  onClick,
}: {
  label: string
  value: ReactNode
  detail: ReactNode
  tone?: "online" | "attention" | "offline" | "muted"
  /** What it sums up is not read yet: the value and its line wait, in their places. */
  loading?: boolean
  onClick?: () => void
}) {
  const body = loading ? (
    <>
      <span className="text-meta text-muted-foreground">{label}</span>
      <span className="flex h-5 items-center">
        <Skeleton className="h-4 w-14 rounded-full" />
      </span>
      <span className="flex h-4 items-center">
        <Skeleton className="h-2.5 w-24 rounded-full" />
      </span>
    </>
  ) : (
    <>
      <span className="text-meta text-muted-foreground">{label}</span>
      <span
        className={cn(
          "truncate text-lead font-medium tabular-nums",
          tone === "online" && "text-success",
          tone === "attention" && "text-warning",
          tone === "offline" && "text-destructive",
          tone === "muted" && "text-muted-foreground"
        )}
      >
        {value}
      </span>
      <span className="truncate text-meta text-muted-foreground">{detail}</span>
    </>
  )
  const className =
    "flex h-[5.25rem] min-w-0 flex-col justify-center gap-0.5 rounded-tile border border-divider px-4 text-left"
  return onClick ? (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        className,
        "transition-colors outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring"
      )}
    >
      {body}
    </button>
  ) : (
    <div className={className}>{body}</div>
  )
}

/** A part shown on demand: one line saying what it holds; its content below once opened. */
export function Disclosure({
  title,
  summary,
  children,
  defaultOpen = false,
}: {
  title: string
  summary?: ReactNode
  children: ReactNode
  defaultOpen?: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <section className="overflow-hidden rounded-tile border border-divider">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="flex min-h-12 w-full items-center gap-3 px-4 text-left outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      >
        <ChevronRightIcon
          aria-hidden="true"
          className={cn(
            "size-4 shrink-0 text-muted-foreground transition-transform",
            open && "rotate-90"
          )}
        />
        <span className="text-row font-medium">{title}</span>
        {summary ? (
          <span className="min-w-0 flex-1 truncate text-right text-meta text-muted-foreground">
            {summary}
          </span>
        ) : null}
      </button>
      {open ? (
        <div className="flex flex-col gap-6 border-t border-divider p-4">
          {children}
        </div>
      ) : null}
    </section>
  )
}
