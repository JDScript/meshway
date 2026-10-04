import { useState, type PointerEvent } from "react"

import { clockTime } from "@/lib/format"
import { millis, percent, rangeEnds } from "@/lib/history/format"
import {
  lineRuns,
  unansweredRate,
  type Series,
  type SeriesPoint,
} from "@/lib/history/series"
import type { HistoryWindow } from "@/lib/history/window"
import { m } from "@/paraglide/messages.js"

// One series over its window, drawn by hand: the mean of each bucket's
// successful probes as a step (a bucket is a span, not an instant), a strip
// below marking the buckets with records and, in the warning colour, those
// where probes timed out or failed. A bucket without records stays blank,
// one where nothing succeeded has no step: neither becomes zero.

const W = 600
const PLOT = 96
const STRIP = 8
const GAP = 6
const H = PLOT + GAP + STRIP

export function LatencyChart({
  series,
  window,
  label,
}: {
  series: Series
  window: HistoryWindow
  label: string
}) {
  const [hovered, setHovered] = useState<SeriesPoint | null>(null)
  const spanMs = window.toMs - window.fromMs
  const x = (ms: number) => ((ms - window.fromMs) / spanMs) * W
  const top = (series.totals.maxUs ?? 1) * 1.15
  const y = (us: number) => PLOT - (us / top) * PLOT
  const bucketMs = window.bucketSeconds * 1000

  const steps = lineRuns(series.points).map((run) => {
    let d = ""
    for (const point of run) {
      const yy = y(point.meanUs ?? 0).toFixed(2)
      d += d === "" ? `M${x(point.fromMs).toFixed(2)},${yy}` : `V${yy}`
      d += `H${x(point.toMs).toFixed(2)}`
    }
    return d
  })

  const onMove = (event: PointerEvent<SVGSVGElement>) => {
    const rect = event.currentTarget.getBoundingClientRect()
    const at =
      window.fromMs + ((event.clientX - rect.left) / rect.width) * spanMs
    const index = Math.floor((at - window.fromMs) / bucketMs)
    setHovered(series.points.find((point) => point.index === index) ?? null)
  }

  const rate = hovered ? unansweredRate(hovered) : null
  return (
    <figure className="flex flex-col gap-1.5">
      <div className="flex h-4 items-center justify-between text-meta text-muted-foreground tabular-nums">
        <span>{series.totals.maxUs !== null ? millis(top) : "—"}</span>
        <span className="min-w-0 truncate">
          {hovered
            ? `${clockTime(hovered.fromMs, { seconds: false })}–${clockTime(hovered.toMs, { seconds: false })} · ${
                hovered.meanUs !== null
                  ? millis(hovered.meanUs)
                  : m.history_no_success()
              } · ${m.history_counts({
                success: String(hovered.success),
                timeout: String(hovered.timeout),
                failed: String(hovered.failed),
              })}${rate !== null && rate > 0 ? ` · ${percent(rate)}` : ""}`
            : ""}
        </span>
      </div>
      <svg
        role="img"
        aria-label={label}
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        className="h-28 w-full touch-none overflow-visible"
        onPointerMove={onMove}
        onPointerLeave={() => setHovered(null)}
      >
        <line
          x1={0}
          x2={W}
          y1={0}
          y2={0}
          className="stroke-divider"
          strokeDasharray="2 4"
          vectorEffect="non-scaling-stroke"
        />
        <line
          x1={0}
          x2={W}
          y1={PLOT}
          y2={PLOT}
          className="stroke-divider"
          vectorEffect="non-scaling-stroke"
        />
        {hovered ? (
          <rect
            x={x(hovered.fromMs)}
            width={Math.max(1, x(hovered.toMs) - x(hovered.fromMs))}
            y={0}
            height={H}
            className="fill-fill-hover"
          />
        ) : null}
        {steps.map((d, i) => (
          <path
            key={i}
            d={d}
            fill="none"
            stroke="var(--signal)"
            strokeWidth={1.5}
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        ))}
        {series.points.map((point) => {
          const unanswered = point.sampleCount - point.success
          const share = point.sampleCount ? unanswered / point.sampleCount : 0
          return (
            <rect
              key={point.index}
              x={x(point.fromMs)}
              width={Math.max(0.5, x(point.toMs) - x(point.fromMs) - 0.5)}
              y={PLOT + GAP}
              height={STRIP}
              style={
                unanswered > 0
                  ? {
                      fill: "var(--warning)",
                      opacity: 0.35 + 0.65 * share,
                    }
                  : { fill: "var(--muted-foreground)", opacity: 0.25 }
              }
            />
          )
        })}
      </svg>
      <figcaption className="flex h-4 items-center justify-between text-meta text-muted-foreground tabular-nums">
        {rangeEnds(window.fromMs, window.toMs).map((end, i) => (
          <span key={i}>{end}</span>
        ))}
      </figcaption>
    </figure>
  )
}
