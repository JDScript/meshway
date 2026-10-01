import { CircleAlertIcon } from "lucide-react"
import type { ReactNode } from "react"

import { DiagnosticDetails } from "@/components/common/diagnostic-details"

/** A real failure: what failed, what to do next, and the raw reason folded away. */
export function FailureNotice({ title, nextStep, error, actions }: {
  title: string
  nextStep: string
  error: string
  actions?: ReactNode
}) {
  return (
    <div role="alert" className="flex min-w-0 gap-3 rounded-lg border border-destructive/25 bg-destructive/[0.04] p-3.5 text-xs">
      <CircleAlertIcon aria-hidden="true" className="mt-px size-4 shrink-0 text-destructive" />
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <div className="flex flex-col gap-0.5">
          <p className="text-[0.8125rem] font-medium text-destructive">{title}</p>
          <p className="leading-relaxed text-muted-foreground">{nextStep}</p>
        </div>
        {actions ? <div className="flex flex-wrap gap-2">{actions}</div> : null}
        <DiagnosticDetails title="错误技术详情" className="bg-card">
          <pre className="max-w-full font-mono whitespace-pre-wrap [overflow-wrap:anywhere]">{error}</pre>
        </DiagnosticDetails>
      </div>
    </div>
  )
}
