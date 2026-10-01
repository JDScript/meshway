import type { ComponentProps, ReactNode } from "react"

import { cn } from "@/lib/utils"

/**
 * One flat surface with an optional header row. Pages compose panels side by
 * side instead of nesting cards inside cards.
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
      className={cn("flex min-w-0 flex-col rounded-xl border bg-card text-card-foreground", className)}
      {...props}
    >
      {title || actions ? (
        <header className="flex flex-wrap items-start justify-between gap-3 border-b px-4 py-3">
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            {title ? <h2 className="text-sm font-medium">{title}</h2> : null}
            {description ? (
              <div className="min-w-0 text-xs text-muted-foreground">{description}</div>
            ) : null}
          </div>
          {actions ? <div className="flex max-w-full shrink-0 flex-wrap items-center gap-1.5">{actions}</div> : null}
        </header>
      ) : null}
      <div className={cn("min-w-0 flex-1", bodyClassName)}>{children}</div>
    </section>
  )
}
