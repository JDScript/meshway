import type { ReactNode } from "react"

/** A compact group of facts: the same row, inset and label column across node views. */
export function FactList({ children }: { children: ReactNode }) {
  return (
    <dl className="min-w-0 divide-y divide-divider overflow-hidden rounded-tile border border-divider">
      {children}
    </dl>
  )
}

export function FactRow({
  label,
  children,
  detail,
  action,
}: {
  label: string
  children: ReactNode
  detail?: ReactNode
  action?: ReactNode
}) {
  return (
    <div className="grid h-16 grid-cols-[6rem_minmax(0,1fr)_auto] items-center gap-3 px-4 py-3 text-row sm:grid-cols-[8rem_minmax(0,1fr)_auto]">
      <dt className="truncate text-meta text-muted-foreground" title={label}>
        {label}
      </dt>
      <dd className="min-w-0">
        <div className="truncate">{children}</div>
        {detail ? (
          <div className="mt-1 truncate text-meta text-muted-foreground">
            {detail}
          </div>
        ) : null}
      </dd>
      <dd className="flex shrink-0 items-center">{action}</dd>
    </div>
  )
}
