import { Tabs as TabsPrimitive } from "@base-ui/react/tabs"
import { cn } from "cn"
import { motion, useReducedMotion } from "motion/react"
import { useId, type ReactNode } from "react"

import { SPRING_LAYOUT } from "@/lib/ease"

// AgentUI's segment pattern: a round track, and a raised pill that glides to
// the chosen segment on a spring. One look for choices and for tab lists.

const TRACK = "inline-flex items-center rounded-full border border-border bg-fill-subtle p-1"
/** Concentric with the track: full round, 4px in. */
const SEGMENT = "relative isolate flex min-h-7 items-center gap-1.5 rounded-full px-3 text-xs font-medium whitespace-nowrap outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring"
const segmentTone = (chosen: boolean) => (chosen ? "text-foreground" : "text-muted-foreground hover:text-foreground")

function SegmentPill({ layoutId }: { layoutId: string }) {
  const reduce = useReducedMotion()
  return (
    <motion.span
      layoutId={layoutId}
      transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
      className="absolute inset-0 -z-10 rounded-full border border-border/70 bg-fill-raised shadow-sm"
    />
  )
}

type Option<T extends string> = { value: T; label: ReactNode }

/** A single choice among a few. */
export function SegmentedControl<T extends string>({
  label,
  value,
  options,
  onChange,
  className,
}: {
  label: string
  value: T
  options: readonly Option<T>[]
  onChange: (value: T) => void
  className?: string
}) {
  const layoutId = useId()
  return (
    <fieldset className={cn(TRACK, className)}>
      <legend className="sr-only">{label}</legend>
      {options.map((option) => {
        const chosen = option.value === value
        return (
          <button key={option.value} type="button" aria-pressed={chosen} onClick={() => onChange(option.value)} className={cn(SEGMENT, segmentTone(chosen))}>
            {chosen ? <SegmentPill layoutId={layoutId} /> : null}
            {option.label}
          </button>
        )
      })}
    </fieldset>
  )
}

/** The same look as a tab list, inside a Base UI / shadcn `Tabs` root. */
export function SegmentedTabs<T extends string>({ value, options, className }: { value: T; options: readonly Option<T>[]; className?: string }) {
  const layoutId = useId()
  return (
    <TabsPrimitive.List className={cn(TRACK, "self-start", className)}>
      {options.map((option) => {
        const chosen = option.value === value
        return (
          <TabsPrimitive.Tab key={option.value} value={option.value} className={cn(SEGMENT, segmentTone(chosen))}>
            {chosen ? <SegmentPill layoutId={layoutId} /> : null}
            {option.label}
          </TabsPrimitive.Tab>
        )
      })}
    </TabsPrimitive.List>
  )
}
