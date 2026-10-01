import type { ConnectionNode } from "@meshway/control-client"

import {
  accessRefLabel,
  pathRole,
  pathRoleLabels,
  resolveAccessNode,
} from "@/lib/access-model"
import { cn } from "@/lib/utils"

/**
 * One grant's exact ordered path. Pure display of the saved or drafted node
 * sequence: nothing here is inferred, measured or claimed to be reachable.
 * Unchosen positions ("") render as placeholders.
 */
export function GrantPath({
  path,
  inventory,
  className,
}: {
  path: readonly string[]
  inventory: ConnectionNode[]
  className?: string
}) {
  return (
    <ol aria-label="授权路径" className={cn("flex flex-wrap items-center gap-y-1.5", className)}>
      {path.map((id, index) => {
        const role = pathRole(index, path.length)
        const ref = id === "" ? null : resolveAccessNode(inventory, id)
        const broken = ref !== null && ref.kind !== "ready"
        return (
          <li key={`${index}-${id}`} className="flex items-center">
            {index > 0 ? (
              <span aria-hidden="true" className="flex items-center px-1">
                <span className="h-px w-3 bg-brand/60" />
                <span className="size-0 border-y-[3px] border-l-[5px] border-y-transparent border-l-brand/60" />
              </span>
            ) : null}
            <span
              className={cn(
                "inline-flex min-w-0 items-center gap-1.5 rounded-md border bg-background px-2 py-1 text-xs",
                role !== "via" && !broken && ref !== null && "border-brand/50",
                broken && "border-destructive/50 text-destructive",
                ref === null && "border-dashed text-muted-foreground"
              )}
            >
              <span className="sr-only">{pathRoleLabels[role]}：</span>
              <span
                aria-hidden="true"
                className={cn(
                  "flex size-4 shrink-0 items-center justify-center rounded-full text-[0.5625rem] font-semibold",
                  role === "via" ? "bg-muted text-muted-foreground" : "bg-brand text-brand-foreground"
                )}
              >
                {pathRoleLabels[role].slice(0, 1)}
              </span>
              <span className="max-w-40 truncate font-medium">
                {ref === null ? "未选择" : accessRefLabel(ref)}
              </span>
            </span>
          </li>
        )
      })}
    </ol>
  )
}
