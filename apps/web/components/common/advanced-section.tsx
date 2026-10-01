"use client"

import { ChevronRightIcon } from "lucide-react"
import { useState, type ReactNode } from "react"

import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { cn } from "@/lib/utils"

/**
 * Progressive disclosure for machine IDs and raw wire fields. It opens by
 * itself whenever something inside needs attention, so nothing invalid is
 * ever hidden.
 */
export function AdvancedSection({
  summary,
  attention = false,
  children,
}: {
  /** Current values shown while collapsed. */
  summary: ReactNode
  attention?: boolean
  children: ReactNode
}) {
  const [open, setOpen] = useState(false)
  const shown = open || attention
  return (
    <Collapsible open={shown} onOpenChange={setOpen} className="flex flex-col">
      <CollapsibleTrigger
        className="flex w-full items-center gap-2 px-4 py-3 text-left text-xs outline-none hover:bg-muted/40 focus-visible:bg-muted/40"
        disabled={attention}
      >
        <ChevronRightIcon
          aria-hidden="true"
          className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", shown && "rotate-90")}
        />
        <span className="font-semibold tracking-wide text-muted-foreground uppercase">高级</span>
        <span className="min-w-0 flex-1 truncate font-mono text-[0.6875rem] text-muted-foreground">{summary}</span>
      </CollapsibleTrigger>
      <CollapsibleContent className="flex flex-col gap-3 px-4 pb-4">{children}</CollapsibleContent>
    </Collapsible>
  )
}
