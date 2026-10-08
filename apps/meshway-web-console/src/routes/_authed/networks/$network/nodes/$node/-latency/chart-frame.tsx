import { cn } from "@meshway/ui/cn"
import type { PointerEvent, ReactNode } from "react"

// The frame both latency charts draw in: a line above naming the scale's
// top and what the pointer is on, the plot over a strip of marks, and the
// two ends of the time shown below. Coordinates are in the viewBox; the SVG
// stretches to its width.

export const CHART_WIDTH = 600
/** Height of the plot area; values grow upwards from its bottom. */
export const PLOT_HEIGHT = 96
const STRIP_GAP = 6
/** Where the strip of marks starts, below the plot. */
export const STRIP_TOP = PLOT_HEIGHT + STRIP_GAP
export const STRIP_HEIGHT = 8
export const CHART_HEIGHT = STRIP_TOP + STRIP_HEIGHT

export function ChartFrame({
  label,
  top,
  pointed,
  ends,
  onPointerMove,
  onPointerLeave,
  children,
}: {
  label: string
  /** The scale's top, in words; "—" without values. */
  top: string
  /** What the pointer is on, in words; empty when nothing. */
  pointed: string
  ends: readonly [string, string]
  onPointerMove: (event: PointerEvent<SVGSVGElement>) => void
  onPointerLeave: () => void
  children: ReactNode
}) {
  return (
    <figure className="flex flex-col gap-1.5">
      <div className="flex h-4 items-center justify-between text-meta text-muted-foreground tabular-nums">
        <span>{top}</span>
        <span className="min-w-0 truncate">{pointed}</span>
      </div>
      <svg
        role="img"
        aria-label={label}
        viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`}
        preserveAspectRatio="none"
        className="h-28 w-full touch-none overflow-visible"
        onPointerMove={onPointerMove}
        onPointerLeave={onPointerLeave}
      >
        <line
          x1={0}
          x2={CHART_WIDTH}
          y1={0}
          y2={0}
          className="stroke-divider"
          strokeDasharray="2 4"
          vectorEffect="non-scaling-stroke"
        />
        <line
          x1={0}
          x2={CHART_WIDTH}
          y1={PLOT_HEIGHT}
          y2={PLOT_HEIGHT}
          className="stroke-divider"
          vectorEffect="non-scaling-stroke"
        />
        {children}
      </svg>
      <figcaption className="flex h-4 items-center justify-between text-meta text-muted-foreground tabular-nums">
        <span>{ends[0]}</span>
        <span>{ends[1]}</span>
      </figcaption>
    </figure>
  )
}

/**
 * One entry of a chart's legend, its swatch as the chart draws it: the
 * line of values, a span of the strip (a bucket), or a mark on it (one
 * probe); spans and marks in the warning colour unless `muted`.
 */
export function ChartLegend({
  swatch,
  muted = false,
  children,
}: {
  swatch: "line" | "span" | "mark"
  muted?: boolean
  children: string
}) {
  return (
    <span className="flex items-center gap-1.5">
      <span
        aria-hidden="true"
        className={cn(
          swatch === "line" && "h-0.5 w-3 rounded-full bg-[var(--signal)]",
          swatch === "span" && "size-2 rounded-[2px]",
          swatch === "mark" && "h-2 w-[3px] rounded-[1px]",
          swatch !== "line" && (muted ? "bg-muted-foreground/70" : "bg-warning")
        )}
      />
      {children}
    </span>
  )
}
