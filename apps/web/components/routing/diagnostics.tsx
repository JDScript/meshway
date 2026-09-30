import type { PolicyDiagnostic } from "@meshway/control-client"
import { CircleAlertIcon, HistoryIcon } from "lucide-react"

import { fieldLabel, type LocalIssue } from "@/lib/policy-model"
import { cn } from "@/lib/utils"

type Item = { key: string; ruleId: string | null; field: string; message: string }

/**
 * Local issues and controller diagnostics. `stale` marks diagnostics that
 * were compiled for an earlier version of the draft.
 */
export function DiagnosticList({
  diagnostics = [],
  issues = [],
  showRule = false,
  stale = false,
  label,
  className,
}: {
  diagnostics?: PolicyDiagnostic[]
  issues?: LocalIssue[]
  showRule?: boolean
  stale?: boolean
  label: string
  className?: string
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
    <ul
      aria-label={label}
      className={cn(
        "flex flex-col gap-1.5 text-xs",
        stale ? "text-muted-foreground" : "text-destructive",
        className
      )}
    >
      {items.map((item) => {
        const Icon = stale && item.key.startsWith("server") ? HistoryIcon : CircleAlertIcon
        return (
          <li key={item.key} className="flex items-start gap-1.5">
            <Icon className="mt-0.5 size-3.5 shrink-0" />
            <span className="min-w-0 break-words">
              {showRule && item.ruleId !== null ? (
                <>
                  <code className="font-mono">{item.ruleId || "（空 ID）"}</code> ·{" "}
                </>
              ) : null}
              <span className="font-medium">{fieldLabel(item.field)}</span>
              {fieldLabel(item.field) !== item.field ? (
                <code className="ml-1 font-mono text-[0.625rem] opacity-60">{item.field}</code>
              ) : null}
              ：{item.message}
            </span>
          </li>
        )
      })}
    </ul>
  )
}
