"use client"

import type { NodeSummary } from "@meshway/control-client"
import type { KeyboardEvent } from "react"

import { cn } from "@/lib/utils"

const WIDTH = 720
const HEIGHT = 440
const CENTER = { x: 320, y: 220 }
const INTERNET = { x: 650, y: 220 }
const NODE_R = 15

type Point = { x: number; y: number }

function shortName(name: string): string {
  const chars = [...name]
  return chars.length > 16 ? `${chars.slice(0, 15).join("")}…` : name
}

/**
 * Deterministic logical layout: active nodes evenly on a ring, ordered by
 * name. Positions carry no geographic or network meaning.
 */
function layout(nodes: NodeSummary[]): Map<string, Point> {
  const sorted = [...nodes].sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
  const radius = sorted.length <= 1 ? 0 : Math.min(170, 70 + sorted.length * 14)
  const positions = new Map<string, Point>()
  sorted.forEach((node, index) => {
    const angle = -Math.PI / 2 + (index / Math.max(sorted.length, 1)) * Math.PI * 2
    positions.set(node.id, {
      x: CENTER.x + radius * Math.cos(angle),
      y: CENTER.y + radius * Math.sin(angle),
    })
  })
  return positions
}

/**
 * Labels sit radially outside the ring, away from the planned-path edges
 * that run through its interior, and carry a halo in the canvas colour so an
 * edge can never visually cross a name.
 */
function labelPlacement(point: Point): {
  x: number
  nameY: number
  ipY: number
  anchor: "start" | "middle" | "end"
} {
  const dx = point.x - CENTER.x
  const dy = point.y - CENTER.y
  const length = Math.hypot(dx, dy)
  const ux = length < 1 ? 0 : dx / length
  const uy = length < 1 ? 1 : dy / length
  const offset = NODE_R + 9
  const x = point.x + ux * offset
  const y = point.y + uy * offset
  const anchor = ux > 0.35 ? "start" : ux < -0.35 ? "end" : "middle"
  if (uy > 0.35) return { x, nameY: y + 8, ipY: y + 20, anchor }
  if (uy < -0.35) return { x, nameY: y - 12, ipY: y, anchor }
  return { x, nameY: y - 1, ipY: y + 11, anchor }
}

const labelHalo = (className: string) =>
  cn("stroke-canvas [paint-order:stroke] [stroke-linejoin:round] [stroke-width:4px]", className)

/** Slightly bowed segment so consecutive hops stay distinguishable. */
function arc(from: Point, to: Point): string {
  const mx = (from.x + to.x) / 2
  const my = (from.y + to.y) / 2
  const dx = to.x - from.x
  const dy = to.y - from.y
  const length = Math.hypot(dx, dy) || 1
  const bow = Math.min(40, length * 0.15)
  const cx = mx - (dy / length) * bow
  const cy = my + (dx / length) * bow
  return `M ${from.x} ${from.y} Q ${cx} ${cy} ${to.x} ${to.y}`
}

export type MapPlan = {
  /** Node IDs in order, exactly as compiled by the controller. */
  path: string[]
  external: boolean
  stale: boolean
}

export function MeshMap({
  nodes,
  selectedId,
  sourceId,
  targetId,
  plan,
  onSelect,
}: {
  nodes: NodeSummary[]
  selectedId: string | null
  sourceId: string
  targetId: string
  plan: MapPlan | null
  onSelect: (id: string) => void
}) {
  const positions = layout(nodes)
  const onPath = new Set(plan?.path ?? [])
  const segments: { key: string; d: string }[] = []
  if (plan) {
    plan.path.forEach((id, index) => {
      const next = plan.path[index + 1]
      const from = positions.get(id)
      const to = next !== undefined ? positions.get(next) : undefined
      if (from && to) segments.push({ key: `${index}-${id}-${next}`, d: arc(from, to) })
    })
    const last = plan.path[plan.path.length - 1]
    const lastPoint = last !== undefined ? positions.get(last) : undefined
    if (plan.external && lastPoint) {
      segments.push({ key: "internet", d: arc(lastPoint, INTERNET) })
    }
  }

  const keyActivate = (id: string) => (event: KeyboardEvent<SVGGElement>) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault()
      onSelect(id)
    }
  }

  return (
    <svg
      viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
      role="group"
      aria-label="逻辑拓扑图：节点按名称排列，连线仅表示规划路径；控制面联系见节点列表与详情"
      className="h-auto w-full select-none"
    >
      <defs>
        <marker
          id="mesh-arrow"
          viewBox="0 0 10 10"
          refX="9"
          refY="5"
          markerWidth="6"
          markerHeight="6"
          orient="auto-start-reverse"
        >
          <path d="M 0 0 L 10 5 L 0 10 z" className="fill-brand" />
        </marker>
      </defs>

      {plan?.external ? (
        <g aria-hidden="true">
          <circle
            cx={INTERNET.x}
            cy={INTERNET.y}
            r={26}
            className={cn("fill-background stroke-brand", plan.stale && "opacity-40")}
            strokeDasharray="4 4"
            strokeWidth={1.5}
          />
          <text x={INTERNET.x} y={INTERNET.y + 4} textAnchor="middle" className="fill-muted-foreground text-[11px]">
            互联网
          </text>
        </g>
      ) : null}

      <g aria-hidden="true" className={plan?.stale ? "opacity-40" : undefined}>
        {segments.map((segment) => (
          <path
            key={segment.key}
            d={segment.d}
            fill="none"
            className="stroke-brand"
            strokeWidth={2.5}
            strokeLinecap="round"
            markerEnd="url(#mesh-arrow)"
            strokeDasharray={segment.key === "internet" ? "6 5" : undefined}
          />
        ))}
      </g>

      {nodes.map((node) => {
        const point = positions.get(node.id)
        if (!point) return null
        const selected = selectedId === node.id
        const isSource = sourceId === node.id
        const isTarget = targetId === node.id
        const highlighted = onPath.has(node.id)
        const label = labelPlacement(point)
        return (
          <g
            key={node.id}
            role="button"
            tabIndex={0}
            aria-label={`节点 ${node.name}${isSource ? "（来源）" : ""}${isTarget ? "（目标）" : ""}${highlighted ? "，在规划路径上" : ""}`}
            aria-pressed={selected}
            onClick={() => onSelect(node.id)}
            onKeyDown={keyActivate(node.id)}
            className="cursor-pointer outline-none [&:focus-visible>circle:first-child]:stroke-ring"
          >
            <circle
              cx={point.x}
              cy={point.y}
              r={NODE_R + 6}
              className={cn(
                "fill-transparent stroke-transparent",
                selected && "stroke-foreground/40"
              )}
              strokeWidth={1.5}
              strokeDasharray={selected ? "3 3" : undefined}
            />
            <circle
              cx={point.x}
              cy={point.y}
              r={NODE_R}
              className={cn(
                "stroke-border transition-colors",
                highlighted && !plan?.stale ? "fill-brand stroke-brand" : "fill-card",
                (isSource || isTarget) && "stroke-brand"
              )}
              strokeWidth={isSource || isTarget ? 2.5 : 1.5}
            />
            <text
              x={label.x}
              y={label.nameY}
              textAnchor={label.anchor}
              className={labelHalo("fill-foreground text-[11px] font-medium")}
            >
              {shortName(node.name)}
            </text>
            <text
              x={label.x}
              y={label.ipY}
              textAnchor={label.anchor}
              className={labelHalo("fill-muted-foreground font-mono text-[9px]")}
            >
              {node.mesh_ipv4}
            </text>
            {isSource || isTarget ? (
              <text
                x={point.x}
                y={point.y + 4}
                textAnchor="middle"
                className={cn(
                  "text-[10px] font-semibold",
                  highlighted && !plan?.stale ? "fill-brand-foreground" : "fill-brand"
                )}
              >
                {isSource ? "源" : "目"}
              </text>
            ) : null}
          </g>
        )
      })}
    </svg>
  )
}
