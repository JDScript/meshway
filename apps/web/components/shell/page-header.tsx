import type { ReactNode } from "react"

export function PageHeader({
  title,
  description,
  actions,
}: {
  title: string
  description?: ReactNode
  actions?: ReactNode
}) {
  return (
    <header data-slot="page-header" className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
      <div className="flex min-w-0 flex-col gap-1">
        <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
        {description ? (
          <p className="max-w-2xl text-sm text-muted-foreground">{description}</p>
        ) : null}
      </div>
      {actions ? (
        <div className="flex flex-wrap items-center gap-2">{actions}</div>
      ) : null}
    </header>
  )
}

/** Shared page width and rhythm for console pages. */
export function PageBody({
  children,
}: {
  children: ReactNode
  /** Compatibility for existing callers; density never changes outer gutters. */
  width?: "default" | "wide"
}) {
  return (
    <div
      data-slot="page-body"
      className="console-page flex min-w-0 w-full flex-col gap-6"
    >
      {children}
    </div>
  )
}
