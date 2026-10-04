import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

/** One labelled fact on a surface; `action` sits at its end (a copy button). */
export function Tile({
  label,
  children,
  tone,
  action,
}: {
  label: string
  children: ReactNode
  tone?: "warning" | "destructive"
  action?: ReactNode
}) {
  return (
    <div className="flex min-w-0 items-center gap-1 rounded-tile bg-fill-subtle py-3 pr-2 pl-3.5">
      <div className="min-w-0 flex-1">
        <div className="text-meta text-muted-foreground">{label}</div>
        <div
          className={cn(
            "mt-1 truncate text-row font-medium",
            tone === "warning" && "text-warning",
            tone === "destructive" && "text-destructive"
          )}
        >
          {children}
        </div>
      </div>
      {action}
    </div>
  )
}
