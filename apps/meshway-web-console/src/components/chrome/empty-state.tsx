import { cn } from "@meshway/ui/cn"
import type { LucideIcon } from "lucide-react"
import type { ReactNode } from "react"

/**
 * Where content would be and there is none (yet): a glyph for what is
 * missing, what that means in a few words, one line on what makes it
 * appear, and the action that does, when there is one. `compact` is a row
 * for inside a list; the full one stands in for a whole section.
 */
export function EmptyState({
  icon: Icon,
  title,
  children,
  action,
  compact = false,
  className,
}: {
  icon: LucideIcon
  title: string
  children?: ReactNode
  action?: ReactNode
  compact?: boolean
  className?: string
}) {
  if (compact)
    return (
      <div
        className={cn(
          "flex min-h-16 items-center gap-3 rounded-tile border border-dashed border-divider px-4 py-3",
          className
        )}
      >
        <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-fill-subtle text-muted-foreground">
          <Icon className="size-4" strokeWidth={1.75} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-row font-medium">{title}</span>
          {children ? (
            <span className="block text-meta text-muted-foreground">
              {children}
            </span>
          ) : null}
        </span>
        {action}
      </div>
    )
  return (
    <div
      className={cn(
        "flex min-h-40 flex-col items-center justify-center gap-1.5 rounded-tile border border-dashed border-divider px-6 py-7 text-center",
        className
      )}
    >
      <span className="mb-1.5 flex size-10 items-center justify-center rounded-full bg-fill-subtle text-muted-foreground">
        <Icon className="size-5" strokeWidth={1.75} />
      </span>
      <p className="text-row font-medium">{title}</p>
      {children ? (
        <p className="max-w-sm text-meta text-muted-foreground">{children}</p>
      ) : null}
      {action ? <div className="mt-2.5">{action}</div> : null}
    </div>
  )
}
