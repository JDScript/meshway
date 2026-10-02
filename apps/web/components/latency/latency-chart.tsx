"use client"

import { useId, useMemo, useState, type KeyboardEvent, type PointerEvent } from "react"

import { formatMs, type LatencyHistory } from "@/lib/latency-history-model"
import { transportLabels } from "@/lib/connections-model"
import { formatClock } from "@/lib/time"
import { cn } from "@/lib/utils"

const W = 600
/** One colour per segment, in order of first appearance; a segment is a different carrier / generation. */
const SEGMENT_COLORS = ["var(--brand)", "var(--info)", "var(--chart-3)", "var(--chart-4)", "var(--chart-2)"]

type Item =
  | { kind: "success"; at: number; ms: number; segment: number }
  | { kind: "timeout" | "failed"; at: number; reason: string | null }

/**
 * Directed RTT history: one line per run of a segment (no line across a
 * segment change or an unrecorded stretch), timeouts and failures as ticks
 * on their own band, segment changes as faint rules. Interactive charts are
 * keyboard-navigable; the readout is an aria-live text, not a hover-only tooltip.
 */
export function LatencyChart({
  history,
  from,
  until,
  height,
  interactive = false,
  failureLabels,
}: {
  history: LatencyHistory
  from: number
  until: number
  height: number
  interactive?: boolean
  failureLabels: Record<string, string>
}) {
  const id = useId()
  const bandH = 10
  const plotTop = 6
  const plotBottom = height - bandH - 6
  const max = Math.max(1, ...history.successes.map((point) => point.ms)) * 1.15
  const x = (at: number) => ((at - from) / Math.max(1, until - from)) * W
  const y = (ms: number) => plotBottom - (ms / max) * (plotBottom - plotTop)
  const segmentIndex = useMemo(() => new Map(history.segments.map((segment, index) => [segment.key, index])), [history.segments])

  const items: Item[] = useMemo(
    () =>
      [
        ...history.successes.map((point): Item => ({ kind: "success", at: point.at, ms: point.ms, segment: segmentIndex.get(point.segment) ?? 0 })),
        ...history.events.map((event): Item => ({ kind: event.outcome, at: event.at, reason: event.reason })),
      ].sort((a, b) => a.at - b.at),
    [history, segmentIndex]
  )
  const [cursor, setCursor] = useState<number | null>(null)
  const active = cursor === null ? null : items[Math.min(cursor, items.length - 1)] ?? null

  const pick = (event: PointerEvent<SVGSVGElement>) => {
    const rect = event.currentTarget.getBoundingClientRect()
    const at = from + ((event.clientX - rect.left) / rect.width) * (until - from)
    let best = -1
    let distance = Number.POSITIVE_INFINITY
    items.forEach((item, index) => {
      const d = Math.abs(item.at - at)
      if (d < distance) {
        distance = d
        best = index
      }
    })
    if (best >= 0) setCursor(best)
  }
  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (items.length === 0) return
    if (event.key === "ArrowRight" || event.key === "ArrowLeft" || event.key === "Home" || event.key === "End") {
      event.preventDefault()
      const current = cursor ?? (event.key === "ArrowLeft" ? items.length : -1)
      const next =
        event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : event.key === "ArrowRight" ? Math.min(items.length - 1, current + 1) : Math.max(0, current - 1)
      setCursor(next)
    } else if (event.key === "Escape") setCursor(null)
  }

  const describe = (item: Item) =>
    item.kind === "success"
      ? `${formatClock(item.at)} · ${formatMs(item.ms)} · ${segmentLabel(history, item.segment)}`
      : `${formatClock(item.at)} · ${item.kind === "timeout" ? "超时" : `失败${item.reason ? `：${failureLabels[item.reason] ?? item.reason}` : ""}`}`

  return (
    <div className="flex flex-col gap-1">
      <div
        role={interactive ? "group" : undefined}
        aria-label={interactive ? "延迟历史图（左右方向键逐个查看记录）" : undefined}
        aria-describedby={interactive ? `${id}-readout` : undefined}
        tabIndex={interactive && items.length > 0 ? 0 : undefined}
        onKeyDown={interactive ? onKey : undefined}
        className={cn("relative rounded-md outline-none", interactive && "focus-visible:ring-2 focus-visible:ring-ring/40")}
      >
        <svg
          viewBox={`0 0 ${W} ${height}`}
          preserveAspectRatio="none"
          aria-hidden="true"
          className="block w-full"
          style={{ height }}
          onPointerMove={interactive ? pick : undefined}
          onPointerLeave={interactive ? () => setCursor(null) : undefined}
        >
          <line x1={0} x2={W} y1={plotBottom} y2={plotBottom} stroke="var(--border)" vectorEffect="non-scaling-stroke" />
          {history.changes.map((change) => (
            <line key={`c${change.at}`} x1={x(change.at)} x2={x(change.at)} y1={plotTop} y2={plotBottom} stroke="var(--muted-foreground)" strokeOpacity={0.35} strokeDasharray="2 3" vectorEffect="non-scaling-stroke" />
          ))}
          {history.segments.map((segment, index) =>
            segment.runs.map((run, runIndex) => (
              <g key={`${segment.key}-${runIndex}`} stroke={SEGMENT_COLORS[index % SEGMENT_COLORS.length]}>
                {run.length > 1 ? (
                  <polyline
                    points={run.map((point) => `${x(point.at)},${y(point.ms)}`).join(" ")}
                    fill="none"
                    strokeWidth={1.6}
                    strokeLinejoin="round"
                    vectorEffect="non-scaling-stroke"
                  />
                ) : null}
                <path
                  d={run.map((point) => `M${x(point.at)} ${y(point.ms)}h0`).join("")}
                  strokeWidth={run.length > 1 ? 3 : 4}
                  strokeLinecap="round"
                  vectorEffect="non-scaling-stroke"
                />
              </g>
            ))
          )}
          {history.events.map((event) => (
            <line
              key={event.id}
              x1={x(event.at)}
              x2={x(event.at)}
              y1={height - bandH}
              y2={height - 2}
              stroke={event.outcome === "timeout" ? "var(--warning)" : "var(--destructive)"}
              strokeWidth={2}
              vectorEffect="non-scaling-stroke"
            />
          ))}
          {active ? (
            <line x1={x(active.at)} x2={x(active.at)} y1={0} y2={height} stroke="var(--foreground)" strokeOpacity={0.5} vectorEffect="non-scaling-stroke" />
          ) : null}
        </svg>
        {interactive ? (
          <span className="pointer-events-none absolute top-0 left-1 text-[0.625rem] text-muted-foreground tabular-nums">{formatMs(max)}</span>
        ) : null}
      </div>
      {interactive ? (
        <div aria-hidden="true" className="flex justify-between text-[0.625rem] text-muted-foreground tabular-nums">
          <span>{formatClock(from)}</span>
          <span>{formatClock(from + (until - from) / 2)}</span>
          <span>{formatClock(until)}</span>
        </div>
      ) : null}
      {interactive ? (
        <p id={`${id}-readout`} aria-live="polite" className="min-h-4 text-xs text-muted-foreground tabular-nums">
          {active ? describe(active) : items.length > 0 ? "悬停或用方向键查看单条记录" : ""}
        </p>
      ) : null}
    </div>
  )
}

export function segmentLabel(history: LatencyHistory, index: number): string {
  const segment = history.segments[index]
  if (!segment) return "—"
  const transport = transportLabels[segment.transport]
  return `${transport} · ${segment.listenerId} · 代际 ${segment.runtimeEpoch} · r${segment.revision} · 采样 ${segment.samplerGeneration.slice(0, 6)}`
}

export { SEGMENT_COLORS }
