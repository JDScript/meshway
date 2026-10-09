import { cn } from "@meshway/ui/cn"

import type { MapConnection } from "@/lib/network-map"
import {
  mapConnectionText,
  mapTransportText,
  transportText,
} from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { lookColor } from "./labels"

export type Point = { x: number; y: number }
export type EdgeDrawing = {
  edge: MapConnection
  a: Point
  b: Point
  control: Point
}
export type EdgeCaption = Point & { width: number; text: string }

/** One actual/configured edge; the wide transparent stroke makes its details
 * reachable without turning the visible line into a heavy hit target.
 */
export function TopologyEdge({
  drawing,
  caption,
  names,
  active,
  highlighted,
  focusable,
  onPoint,
  onFocus,
}: {
  drawing: EdgeDrawing
  caption: EdgeCaption | undefined
  names: (id: string) => string
  active: boolean
  highlighted: boolean
  focusable: boolean
  onPoint: (key: string | null) => void
  onFocus: (key: string | null) => void
}) {
  const { edge, a, b, control } = drawing
  const path = `M ${a.x} ${a.y} Q ${control.x} ${control.y} ${b.x} ${b.y}`
  const title = [
    ...new Set(
      [
        `${names(edge.a)} ↔ ${names(edge.b)}`,
        edge.status === "history" ||
        edge.status === "connecting" ||
        edge.status === "failed"
          ? null
          : mapConnectionText(edge),
        mapTransportText(edge),
        edge.declaration
          ? edge.declaration.rule === null
            ? m.overview_topology_default()
            : m.overview_topology_rule({ number: edge.declaration.rule + 1 })
          : null,
        edge.transports.length && edge.declaration?.transports?.length
          ? m.overview_allowed_transports({
              value: edge.declaration.transports.map(transportText).join(" / "),
            })
          : null,
      ].filter(Boolean)
    ),
  ].join(" · ")
  return (
    <g
      onPointerEnter={() => onPoint(edge.key)}
      onPointerLeave={() => onPoint(null)}
    >
      <path
        d={path}
        fill="none"
        stroke={lookColor[edge.look]}
        strokeWidth={highlighted ? 2.5 : edge.look === "matched" ? 2 : 1.5}
        strokeDasharray={
          edge.look === "matched" || edge.look === "history" ? undefined : "6 6"
        }
        opacity={
          highlighted
            ? 1
            : (edge.look === "history" ? 0.25 : 0.8) * (active ? 1 : 0.2)
        }
        aria-hidden="true"
      />
      <path
        d={path}
        fill="none"
        stroke="transparent"
        strokeWidth={14}
        pointerEvents="stroke"
        tabIndex={focusable ? 0 : -1}
        role="img"
        aria-label={title}
        className="outline-none"
        onFocus={() => onFocus(edge.key)}
        onBlur={() => onFocus(null)}
      >
        <title>{title}</title>
      </path>
      {caption ? (
        <foreignObject
          x={caption.x - caption.width / 2}
          y={caption.y - 13}
          width={caption.width}
          height={26}
          className="pointer-events-none overflow-visible"
          opacity={active || highlighted ? 1 : 0.3}
        >
          <div className="flex h-full items-center justify-center">
            <span
              className={cn(
                "max-w-full truncate rounded-full border bg-background px-2.5 py-1 text-meta leading-4",
                highlighted
                  ? "border-ring text-foreground"
                  : "border-border text-muted-foreground",
                edge.status === "connected" && "text-foreground"
              )}
            >
              {caption.text}
            </span>
          </div>
        </foreignObject>
      ) : null}
    </g>
  )
}
