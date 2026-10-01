"use client"

import { ChevronRightIcon } from "lucide-react"
import { useState, type ReactNode } from "react"

import { cn } from "@/lib/utils"

/** Disclosure never unmounts children: clocks, copy eligibility and drafts keep their lifetime. */
export function DiagnosticDetails({ title = "高级技术详情", children, className, id, open }: {
  title?: string
  children: ReactNode
  className?: string
  id?: string
  /** Request expansion on a rising change (hash target, restored draft, error); never discard a user's open view. */
  open?: boolean
}) {
  const [state, setState] = useState({ requested: open, expanded: Boolean(open) })
  if (state.requested !== open) {
    setState({ requested: open, expanded: Boolean(open) || state.expanded })
  }
  return (
    <details id={id} open={state.expanded} onToggle={(event) => {
      const expanded = event.currentTarget.open
      setState((current) => current.expanded === expanded ? current : { ...current, expanded })
    }} data-slot="diagnostic-details" className={cn("group/diagnostics min-w-0 rounded-md border", className)}>
      <summary className="flex cursor-pointer list-none items-center gap-2 rounded-md px-3.5 py-2.5 text-xs font-medium text-muted-foreground select-none hover:bg-muted/50 hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden">
        <ChevronRightIcon aria-hidden="true" className="size-3.5 shrink-0 transition-transform group-open/diagnostics:rotate-90" />
        {title}
      </summary>
      <div className="flex min-w-0 flex-col gap-3 px-3.5 pb-3.5 text-xs [overflow-wrap:anywhere]">{children}</div>
    </details>
  )
}
