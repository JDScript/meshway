import { cn } from "@/lib/utils"
import type { ReactNode } from "react"

const TITLE = { md: "text-xl", lg: "text-2xl" } as const

/**
 * The head of a peek or a workspace: the title and the close control share
 * one centred line; facts about the entity sit below. The header uses the
 * surface's own inset, the same as the content under it.
 */
export function SurfaceHeader({
  title,
  meta,
  action,
  size = "md",
  className,
}: {
  /** Render the heading element (an `h2`, or a dialog's title) with the given classes. */
  title: (className: string) => ReactNode
  meta?: ReactNode
  action?: ReactNode
  size?: keyof typeof TITLE
  className?: string
}) {
  return (
    <header className={cn("flex flex-col gap-1.5", className)}>
      <div className="flex min-w-0 items-center gap-3">
        {title(
          cn(
            "min-w-0 flex-1 truncate font-sans font-semibold tracking-tight",
            TITLE[size]
          )
        )}
        {action}
      </div>
      {meta}
    </header>
  )
}
