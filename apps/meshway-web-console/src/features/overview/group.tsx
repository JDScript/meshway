import { ChevronDownIcon } from "lucide-react"
import { useState, type ReactNode } from "react"

import { cn } from "@/lib/utils"

/** A titled part of the card that folds. */
export function Group({
  title,
  count,
  note,
  children,
  defaultOpen = true,
}: {
  title: string
  count?: number
  note?: string
  children: ReactNode
  defaultOpen?: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <section className="flex flex-col gap-1">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="-mx-1 flex h-7 items-center gap-1.5 rounded-full px-1 text-left text-meta font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ChevronDownIcon
          className={cn(
            "size-3.5 transition-transform",
            open ? "" : "-rotate-90"
          )}
        />
        {title}
        {count !== undefined ? (
          <span className="tabular-nums">{count}</span>
        ) : null}
        {note ? (
          <span className="min-w-0 flex-1 truncate text-right font-normal">
            {note}
          </span>
        ) : null}
      </button>
      {open ? children : null}
    </section>
  )
}
