import { cn } from "@/lib/utils"
import type { ReactNode } from "react"

/**
 * A titled part of a workspace (a group of fields, a tab's list): the
 * heading, how many of what it names, and one action at its end, on a line
 * of one height whether or not the action is there.
 */
export function Section({
  title,
  count,
  action,
  children,
  className,
}: {
  title: string
  /** How many of what the title names; null while unread (the slot stays). */
  count?: string | number | null
  action?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <section className={cn("flex flex-col gap-2", className)}>
      <div className="flex h-8 items-center gap-2">
        <h3 className="text-row font-medium text-muted-foreground">{title}</h3>
        {count !== undefined ? (
          <span className="text-meta text-muted-foreground tabular-nums">
            {count}
          </span>
        ) : null}
        <span className="flex-1" />
        {action}
      </div>
      {children}
    </section>
  )
}

const EMPTY_SIZE = { md: "h-12", sm: "h-10" } as const

/**
 * The fixed-height line where a section's content would be: empty, not
 * read, or unknown. Short facts ("没有节点"). `dashed` for a line already
 * on a fill (inside a card).
 */
export function EmptyLine({
  size = "md",
  dashed = false,
  children,
}: {
  size?: keyof typeof EMPTY_SIZE
  dashed?: boolean
  children: ReactNode
}) {
  return (
    <div
      className={cn(
        "flex items-center rounded-tile px-3.5 text-row text-muted-foreground",
        EMPTY_SIZE[size],
        dashed ? "border border-dashed border-divider" : "bg-fill-subtle"
      )}
    >
      {children}
    </div>
  )
}

/**
 * The tile before something that cannot be undone (revoking a node,
 * removing a member): what stands now, and the control at its end. What
 * the action does is stated in its confirmation.
 */
export function DangerRow({
  fact,
  action,
}: {
  fact: ReactNode
  action: ReactNode
}) {
  return (
    <div className="flex min-h-12 items-center gap-3 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
      <span className="min-w-0 flex-1 truncate text-row">{fact}</span>
      {action}
    </div>
  )
}
