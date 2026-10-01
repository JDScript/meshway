"use client"

import { InfoIcon } from "lucide-react"
import type { ReactNode } from "react"

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { cn } from "@/lib/utils"

/**
 * The meaning of a label, one tap / hover / focus away. Boundaries such as
 * "contact is not liveness" live here instead of repeating under every title.
 * Opens on click as well as hover, so touch screens can read it.
 */
export function Hint({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <Popover>
      <PopoverTrigger
        openOnHover
        delay={150}
        aria-label={`${label}：说明`}
        className={cn(
          "inline-flex size-4 shrink-0 items-center justify-center rounded-full align-middle text-muted-foreground/70 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40",
          className
        )}
      >
        <InfoIcon className="size-3.5" aria-hidden="true" />
      </PopoverTrigger>
      <PopoverContent side="top" className="w-64 gap-1 text-xs leading-relaxed text-popover-foreground">
        {children}
      </PopoverContent>
    </Popover>
  )
}

/** A label followed by its hint, for column headers and field labels. */
export function HintLabel({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1">
      {label}
      <Hint label={label}>{children}</Hint>
    </span>
  )
}
