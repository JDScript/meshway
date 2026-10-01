import type { ReactNode } from "react"

export function SectionHeader({ title, description, actions, id }: {
  title: string
  description?: ReactNode
  actions?: ReactNode
  id?: string
}) {
  return <div data-slot="section-header" className="flex min-w-0 flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
    <div className="flex min-w-0 flex-col gap-0.5">
      <h2 id={id} className="text-sm font-medium">{title}</h2>
      {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
    </div>
    {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
  </div>
}
