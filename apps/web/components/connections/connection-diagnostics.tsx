import type { ConnectionDiagnostic } from "@meshway/control-client"
import { CircleAlertIcon, HistoryIcon } from "lucide-react"

import { connectionFieldLabel, type ConnectionIssue } from "@/lib/connections-model"
import { cn } from "@/lib/utils"

/**
 * Local draft issues and controller diagnostics. `stale` marks diagnostics
 * the controller reported for the saved source, which the draft has since
 * changed.
 */
export function ConnectionDiagnosticList({
  diagnostics = [],
  issues = [],
  showItem = false,
  stale = false,
  label,
  className,
}: {
  diagnostics?: ConnectionDiagnostic[]
  issues?: ConnectionIssue[]
  showItem?: boolean
  stale?: boolean
  label: string
  className?: string
}) {
  const items: { key: string; server: boolean; itemId: string | null; field: string; message: string }[] = [
    ...issues.map((issue, index) => ({
      key: `local-${index}`,
      server: false,
      itemId: null,
      field: issue.field,
      message: issue.message,
    })),
    ...diagnostics.map((diagnostic, index) => ({
      key: `server-${index}`,
      server: true,
      itemId: diagnostic.item_id,
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
        const Icon = stale && item.server ? HistoryIcon : CircleAlertIcon
        return (
          <li key={item.key} className="flex items-start gap-1.5">
            <Icon className="mt-0.5 size-3.5 shrink-0" />
            <span className="min-w-0 break-words">
              {showItem && item.itemId !== null ? (
                <>
                  <code className="font-mono">{item.itemId}</code> ·{" "}
                </>
              ) : null}
              <span className="font-medium">{connectionFieldLabel(item.field)}</span>
              {item.server ? (
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

/** Controller diagnostics that belong to one listener or candidate ID. */
export function diagnosticsFor(
  diagnostics: ConnectionDiagnostic[],
  collection: "listeners" | "candidates",
  id: string
): ConnectionDiagnostic[] {
  return diagnostics.filter(
    (diagnostic) =>
      diagnostic.item_id === id &&
      (diagnostic.field === collection || diagnostic.field.startsWith(`${collection}.`))
  )
}
