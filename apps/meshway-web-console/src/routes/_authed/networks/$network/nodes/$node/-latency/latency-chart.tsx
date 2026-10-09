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

import {
  CHART_HEIGHT,
  CHART_WIDTH,
  ChartFrame,
  PLOT_HEIGHT,
  STRIP_HEIGHT,
  STRIP_TOP,
} from "./chart-frame"

// One series over its window, drawn by hand: the mean of each bucket's
// successful probes as a step (a bucket is a span, not an instant), a strip
// below marking the buckets with records and, in the warning colour, those
// where probes timed out or failed. A bucket without records stays blank,
// one where nothing succeeded has no step: neither becomes zero.

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
  const x = (ms: number) => ((ms - window.fromMs) / spanMs) * CHART_WIDTH
  const top = (series.totals.maxUs ?? 1) * 1.15
  const y = (us: number) => PLOT_HEIGHT - (us / top) * PLOT_HEIGHT
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
    <ChartFrame
      label={label}
      top={series.totals.maxUs !== null ? millis(top) : "—"}
      pointed={
        hovered
          ? `${clockTime(hovered.fromMs, { seconds: false })}–${clockTime(hovered.toMs, { seconds: false })} · ${
              hovered.meanUs !== null
                ? millis(hovered.meanUs)
                : m.history_no_success()
            } · ${m.history_counts({
              success: String(hovered.success),
              timeout: String(hovered.timeout),
              failed: String(hovered.failed),
            })}${rate !== null && rate > 0 ? ` · ${percent(rate)}` : ""}`
          : ""
      }
      ends={rangeEnds(window.fromMs, window.toMs)}
      onPointerMove={onMove}
      onPointerLeave={() => setHovered(null)}
    >
      {hovered ? (
        <rect
          x={x(hovered.fromMs)}
          width={Math.max(1, x(hovered.toMs) - x(hovered.fromMs))}
          y={0}
          height={CHART_HEIGHT}
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
            y={STRIP_TOP}
            height={STRIP_HEIGHT}
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
    </ChartFrame>
  )
}
