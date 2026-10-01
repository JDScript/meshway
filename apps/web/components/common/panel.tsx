import type { ComponentProps, ReactNode } from "react"

import { cn } from "@/lib/utils"

/**
 * The console's one card surface: an optional header row, then content.
 * Pages lay panels side by side instead of nesting cards inside cards.
 */
export function Panel({
  title,
  description,
  actions,
  children,
  className,
  bodyClassName,
  ...props
}: Omit<ComponentProps<"section">, "title"> & {
  title?: ReactNode
  description?: ReactNode
  actions?: ReactNode
  bodyClassName?: string
}) {
  return (
    <section
      data-slot="panel"
      className={cn("flex min-w-0 flex-col rounded-lg border bg-card text-card-foreground shadow-xs", className)}
      {...props}
    >
      {title || actions ? (
        <header className="flex flex-wrap items-start justify-between gap-3 border-b px-4 py-3 sm:px-5">
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            {title ? <h2 className="text-sm font-semibold tracking-tight">{title}</h2> : null}
            {description ? <div className="min-w-0 text-xs leading-relaxed text-muted-foreground">{description}</div> : null}
          </div>
          {actions ? <div className="flex max-w-full shrink-0 flex-wrap items-center gap-1.5">{actions}</div> : null}
        </header>
      ) : null}
      <div className={cn("min-w-0 flex-1", bodyClassName)}>{children}</div>
    </section>
  )
}

/** Label / value rows inside a panel. */
export function Fields({ children, className }: { children: ReactNode; className?: string }) {
  return <dl className={cn("divide-y", className)}>{children}</dl>
}

export function Field({
  label,
  hint,
  children,
  className,
}: {
  label: ReactNode
  hint?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        "grid grid-cols-[7rem_minmax(0,1fr)] items-start gap-3 px-4 py-2.5 sm:grid-cols-[9rem_minmax(0,1fr)] sm:px-5",
        className
      )}
    >
      <dt className="flex flex-col gap-0.5 pt-px text-xs text-muted-foreground">
        {label}
        {hint ? <span className="text-[0.6875rem] leading-snug text-muted-foreground/80">{hint}</span> : null}
      </dt>
      <dd className="min-w-0 text-[0.8125rem]">{children}</dd>
    </div>
  )
}
