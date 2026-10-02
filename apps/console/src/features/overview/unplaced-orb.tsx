import { cn } from "cn"
import { useState, type CSSProperties } from "react"

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import type { SceneNode, Vitality } from "@/features/globe/scene"
import { m } from "@/paraglide/messages.js"

import { vitalityColor, vitalityLabel } from "./labels"
import { VitalityDot } from "./status-dots"

const ORDER: Vitality[] = ["live", "contact", "quiet"]
const SIZE = 48
const RING = 20.5
const STROKE = 2.5
const GAP = 0.06

/** The ring around the count: one arc per state, in proportion. */
function StateRing({ nodes }: { nodes: readonly SceneNode[] }) {
  const circumference = 2 * Math.PI * RING
  const present = ORDER.map((state) => ({ state, count: nodes.filter((node) => node.vitality === state).length })).filter(
    (entry) => entry.count > 0
  )
  const gap = present.length > 1 ? circumference * GAP : 0
  const shares = present.map(({ count }) => (count / nodes.length) * circumference)
  const arcs = present.map(({ state }, index) => ({
    state,
    length: Math.max((shares[index] ?? 0) - gap, 0.5),
    offset: shares.slice(0, index).reduce((sum, share) => sum + share, 0),
  }))
  return (
    <svg viewBox={`0 0 ${SIZE} ${SIZE}`} aria-hidden="true" className="absolute inset-0 size-full -rotate-90">
      <circle cx={SIZE / 2} cy={SIZE / 2} r={RING} fill="none" stroke="currentColor" strokeOpacity={0.08} strokeWidth={STROKE} />
      {arcs.map((arc) => (
        <circle
          key={arc.state}
          cx={SIZE / 2}
          cy={SIZE / 2}
          r={RING}
          fill="none"
          stroke={vitalityColor[arc.state]}
          strokeWidth={STROKE}
          strokeLinecap="round"
          strokeDasharray={`${arc.length} ${circumference}`}
          strokeDashoffset={-arc.offset}
          className="transition-[stroke-dasharray,stroke-dashoffset] duration-700"
        />
      ))}
    </svg>
  )
}

/**
 * Nodes without a sourced location: the globe has no place for them, so a
 * round button beside it counts them (ringed by their states) and opens the
 * list.
 */
export function UnplacedOrb({
  nodes,
  selectedId,
  onSelect,
  className,
  style,
}: {
  nodes: readonly SceneNode[]
  selectedId: string | null
  onSelect: (nodeId: string) => void
  className?: string
  style?: CSSProperties
}) {
  const [open, setOpen] = useState(false)
  const sorted = [...nodes].sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
  const label = `${m.unplaced_title()} ${nodes.length}`
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        aria-label={label}
        className={cn(
          "glass pointer-events-auto flex items-center justify-center rounded-full outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
          className
        )}
        style={{ width: SIZE, height: SIZE, ...style }}
      >
        <StateRing nodes={nodes} />
        <span className="relative text-sm font-semibold tabular-nums">{nodes.length}</span>
      </PopoverTrigger>
      <PopoverContent side="top" align="start" sideOffset={10} className="glass w-64 gap-0 rounded-surface p-2">
        <header className="flex items-baseline justify-between px-2 pt-1.5 pb-1.5">
          <h2 className="text-xs text-muted-foreground">{m.unplaced_title()}</h2>
          <span className="text-xs text-muted-foreground tabular-nums">{nodes.length}</span>
        </header>
        <ul className="max-h-[min(50svh,320px)] overflow-y-auto">
          {sorted.map((node) => (
            <li key={node.id}>
              <button
                type="button"
                aria-current={node.id === selectedId ? "true" : undefined}
                onClick={() => {
                  setOpen(false)
                  onSelect(node.id)
                }}
                className="list-row h-10 gap-2.5 px-2 text-row"
              >
                <VitalityDot vitality={node.vitality} />
                <span className="min-w-0 flex-1 truncate">{node.name}</span>
                <span className="shrink-0 text-meta text-muted-foreground">{vitalityLabel[node.vitality]()}</span>
              </button>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  )
}
