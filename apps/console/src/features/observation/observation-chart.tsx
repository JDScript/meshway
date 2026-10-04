import { useState, type CSSProperties, type PointerEvent } from "react"

import { millis } from "@/features/history/format"
import { clockTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { replyRuns, type SeriesPoint } from "./series"

// The last stretch of an observation, drawn by hand: each timed reply a
// point on its round trip, consecutive ones joined while they share a path
// segment and no sample is missing between them; a strip below marks every
// timeout (warning), unsent probe and open window. Time without a sample is
// blank, and nothing is drawn at zero.

const W = 600
const PLOT = 96
const STRIP = 8
const GAP = 6
const H = PLOT + GAP + STRIP

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
  const x = (ms: number) => ((ms - fromMs) / spanMs) * W
  const timed = shown.flatMap((p) => (p.us === null ? [] : [p.us]))
  const top = timed.length ? Math.max(...timed) * 1.15 : null
  const y = (us: number) => PLOT - (us / (top ?? 1)) * PLOT

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
    <figure className="flex flex-col gap-1.5">
      <div className="flex h-4 items-center justify-between text-meta text-muted-foreground tabular-nums">
        <span>{top !== null ? millis(top) : "—"}</span>
        <span className="min-w-0 truncate">
          {hovered
            ? `${clockTime(hovered.atMs)} · ${
                hovered.us !== null
                  ? millis(hovered.us)
                  : outcomeText[hovered.outcome]()
              }`
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
        {segmentStarts.map((p, i) => (
          <line
            key={`s${i}`}
            x1={x(p.atMs)}
            x2={x(p.atMs)}
            y1={0}
            y2={H}
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
            y2={H}
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
              y={PLOT + GAP}
              height={STRIP}
              style={markStyle[p.outcome]}
            />
          )
        )}
      </svg>
      <figcaption className="flex h-4 items-center justify-between text-meta text-muted-foreground tabular-nums">
        <span>{clockTime(fromMs)}</span>
        <span>{clockTime(toMs)}</span>
      </figcaption>
    </figure>
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
