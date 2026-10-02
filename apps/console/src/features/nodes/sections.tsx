import type { ReactNode } from "react"

/** A section of a workspace tab: a heading with its count and an action. */
export function TabSection({
  title,
  count,
  action,
  children,
}: {
  title: string
  count?: string | null
  action?: ReactNode
  children: ReactNode
}) {
  return (
    <section className="flex flex-col gap-2">
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

/** A fixed-height line for an empty, failed or unknown section. */
export function EmptyLine({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-12 items-center rounded-tile bg-fill-subtle px-3.5 text-row text-muted-foreground">
      {children}
    </div>
  )
}
