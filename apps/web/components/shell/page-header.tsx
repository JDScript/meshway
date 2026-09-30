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
    <header className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
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
  width = "default",
}: {
  children: ReactNode
  width?: "default" | "wide"
}) {
  return (
    <div
      className={
        width === "wide"
          ? "mx-auto flex w-full max-w-[96rem] flex-col gap-6 px-4 py-6 md:px-6 md:py-8"
          : "mx-auto flex w-full max-w-6xl flex-col gap-6 px-4 py-6 md:px-6 md:py-8"
      }
    >
      {children}
    </div>
  )
}
