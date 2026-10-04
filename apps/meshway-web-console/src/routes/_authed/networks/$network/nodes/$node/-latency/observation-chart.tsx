import { useState, type CSSProperties, type PointerEvent } from "react"

import { clockTime } from "@/lib/format"
import { millis } from "@/lib/history/format"
import { replyRuns, type SeriesPoint } from "@/lib/observation/series"
import { m } from "@/paraglide/messages.js"

import {
  CHART_HEIGHT,
  CHART_WIDTH,
  ChartFrame,
  PLOT_HEIGHT,
  STRIP_HEIGHT,
  STRIP_TOP,
} from "./chart-frame"

// The last stretch of an observation, drawn by hand: each timed reply a
// point on its round trip, consecutive ones joined while they share a path
// segment and no sample is missing between them; a strip below marks every
// timeout (warning), unsent probe and open window. Time without a sample is
// blank, and nothing is drawn at zero.

export function ObservationChart({
  points,
  fromMs,
  toMs,
  gapMs,
  label,
}: {
  points: readonly SeriesPoint[]
  fromMs: number
  toMs: number
  gapMs: number
  label: string
}) {
  const [hovered, setHovered] = useState<SeriesPoint | null>(null)
  const shown = points.filter((p) => p.atMs >= fromMs && p.atMs <= toMs)
  const spanMs = toMs - fromMs
  const x = (ms: number) => ((ms - fromMs) / spanMs) * CHART_WIDTH
  const timed = shown.flatMap((p) => (p.us === null ? [] : [p.us]))
  const top = timed.length ? Math.max(...timed) * 1.15 : null
  const y = (us: number) => PLOT_HEIGHT - (us / (top ?? 1)) * PLOT_HEIGHT

  const runs = replyRuns(shown, gapMs)
  const segmentStarts = shown.filter(
    (p, i) => i > 0 && shown[i - 1]!.segment !== p.segment
  )

  const onMove = (event: PointerEvent<SVGSVGElement>) => {
    const rect = event.currentTarget.getBoundingClientRect()
    const at = fromMs + ((event.clientX - rect.left) / rect.width) * spanMs
    let nearest: SeriesPoint | null = null
    for (const point of shown)
      if (!nearest || Math.abs(point.atMs - at) < Math.abs(nearest.atMs - at))
        nearest = point
    setHovered(nearest && Math.abs(nearest.atMs - at) <= gapMs ? nearest : null)
  }

  return (
    <ChartFrame
      label={label}
      top={top !== null ? millis(top) : "—"}
      pointed={
        hovered
          ? `${clockTime(hovered.atMs)} · ${
              hovered.us !== null
                ? millis(hovered.us)
                : outcomeText[hovered.outcome]()
            }`
          : ""
      }
      ends={[clockTime(fromMs), clockTime(toMs)]}
      onPointerMove={onMove}
      onPointerLeave={() => setHovered(null)}
    >
      {segmentStarts.map((p, i) => (
        <line
          key={`s${i}`}
          x1={x(p.atMs)}
          x2={x(p.atMs)}
          y1={0}
          y2={CHART_HEIGHT}
          className="stroke-muted-foreground/40"
          strokeDasharray="3 3"
          vectorEffect="non-scaling-stroke"
        />
      ))}
      {hovered ? (
        <line
          x1={x(hovered.atMs)}
          x2={x(hovered.atMs)}
          y1={0}
          y2={CHART_HEIGHT}
          className="stroke-foreground/30"
          vectorEffect="non-scaling-stroke"
        />
      ) : null}
      {runs.map((run, i) =>
        run.length > 1 ? (
          <path
            key={i}
            d={run
              .map(
                (p, j) =>
                  `${j ? "L" : "M"}${x(p.atMs).toFixed(2)},${y(p.us ?? 0).toFixed(2)}`
              )
              .join("")}
            fill="none"
            stroke="var(--signal)"
            strokeWidth={1.5}
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        ) : null
      )}
      {shown.map((p, i) =>
        p.us !== null ? (
          <line
            key={`p${i}`}
            x1={x(p.atMs)}
            x2={x(p.atMs)}
            y1={y(p.us)}
            y2={y(p.us)}
            stroke="var(--signal)"
            strokeWidth={5}
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
          />
        ) : null
      )}
      {shown.map((p, i) =>
        p.outcome === "success" && p.us !== null ? null : (
          <rect
            key={`m${i}`}
            x={x(p.atMs) - 1.5}
            width={3}
            y={STRIP_TOP}
            height={STRIP_HEIGHT}
            style={markStyle[p.outcome]}
          />
        )
      )}
    </ChartFrame>
  )
}

const outcomeText: Record<SeriesPoint["outcome"], () => string> = {
  success: () => m.observation_untimed(),
  timeout: () => m.observation_outcome_timeout(),
  failed: () => m.observation_outcome_failed(),
  pending: () => m.observation_outcome_pending(),
}

const markStyle: Record<SeriesPoint["outcome"], CSSProperties> = {
  success: { fill: "var(--muted-foreground)", opacity: 0.4 },
  timeout: { fill: "var(--warning)" },
  failed: { fill: "var(--muted-foreground)", opacity: 0.7 },
  pending: {
    fill: "none",
    stroke: "var(--muted-foreground)",
    strokeWidth: 1,
    opacity: 0.6,
  },
}
