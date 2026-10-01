import type { ReactNode } from "react"

import { DiagnosticDetails } from "@/components/common/diagnostic-details"

export function FailureNotice({ title, nextStep, error, actions }: {
  title: string
  nextStep: string
  error: string
  actions?: ReactNode
}) {
  return (
    <div role="alert" className="flex min-w-0 flex-col gap-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs">
      <p className="font-medium text-destructive">{title}</p>
      <p className="text-muted-foreground">{nextStep}</p>
      {actions ? <div className="flex flex-wrap gap-2">{actions}</div> : null}
      <DiagnosticDetails title="错误技术详情">
        <pre className="max-w-full font-mono whitespace-pre-wrap [overflow-wrap:anywhere]">{error}</pre>
      </DiagnosticDetails>
    </div>
  )
}
