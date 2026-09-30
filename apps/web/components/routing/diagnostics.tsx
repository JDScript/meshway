import type { PolicyDiagnostic } from "@meshway/control-client"
import { CircleAlertIcon } from "lucide-react"

import { fieldLabel, type LocalIssue } from "@/lib/policy-model"

type Item = { key: string; ruleId: string | null; field: string; message: string }

export function DiagnosticList({
  diagnostics = [],
  issues = [],
  showRule = false,
  label,
}: {
  diagnostics?: PolicyDiagnostic[]
  issues?: LocalIssue[]
  showRule?: boolean
  label: string
}) {
  const items: Item[] = [
    ...issues.map((issue, index) => ({
      key: `local-${index}`,
      ruleId: null,
      field: issue.field,
      message: issue.message,
    })),
    ...diagnostics.map((diagnostic, index) => ({
      key: `server-${index}`,
      ruleId: diagnostic.rule_id,
      field: diagnostic.field,
      message: diagnostic.message,
    })),
  ]
  if (items.length === 0) return null

  return (
    <ul aria-label={label} className="flex flex-col gap-1.5 text-xs text-destructive">
      {items.map((item) => (
        <li key={item.key} className="flex items-start gap-1.5">
          <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 break-words">
            {showRule && item.ruleId !== null ? (
              <>
                规则 <code className="font-mono">{item.ruleId || "（空 ID）"}</code> ·{" "}
              </>
            ) : null}
            <span className="font-medium">{fieldLabel(item.field)}</span>{" "}
            <code className="font-mono text-[0.6875rem] opacity-70">{item.field}</code>
            ：{item.message}
          </span>
        </li>
      ))}
    </ul>
  )
}
