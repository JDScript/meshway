import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

/** Saved intent and editing base remain visible; wire diagnostics belong in disclosure. */
export function StatusSummary({ children }: { children: ReactNode }) {
  return <div data-slot="status-summary" className="flex min-w-0 flex-wrap items-center gap-x-5 gap-y-2 rounded-lg border bg-card px-4 py-2.5 text-xs shadow-xs">{children}</div>
}

export function Metric({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return <span className={cn("flex min-w-0 flex-wrap items-center gap-1.5", className)}><span className="text-muted-foreground">{label}</span>{children}</span>
}
