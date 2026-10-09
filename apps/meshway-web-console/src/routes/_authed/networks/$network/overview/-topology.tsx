import { Button } from "@meshway/ui/button"
import { MinusIcon, PlusIcon, ScanIcon, ServerIcon } from "lucide-react"
import { useEffect, useRef, useState } from "react"

import { NodeStatusDot } from "@/components/chrome/node-status"
import type { DeclaredTopology } from "@/lib/configuration/topology-view"
import type { MapConnection, MapNode } from "@/lib/network-map"
import {
  mapConnectionText,
  statusLabel,
  transportText,
} from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { lookColor } from "./-labels"

/** Stable slots depend on node identities, never report time, status or a transient connection. */
function positions(ids: readonly string[], width: number, height: number) {
  const result = new Map<string, { x: number; y: number }>()
  const columns = Math.max(1, Math.floor(width / 220))
  ids.forEach((id, index) => {
    if (ids.length <= 8 && width >= 600) {
      const angle =
        (ids.length === 2 ? 0 : -Math.PI / 2) +
        (index * Math.PI * 2) / Math.max(ids.length, 1)
      result.set(
        id,
        ids.length === 1
          ? { x: width / 2, y: height / 2 }
          : {
              x: width / 2 + Math.cos(angle) * Math.min(220, width / 2 - 125),
              y: height / 2 + Math.sin(angle) * Math.min(160, height / 2 - 100),
            }
      )
    } else
      result.set(id, {
        x: (width / columns) * ((index % columns) + 0.5),
        y: 100 + Math.floor(index / columns) * 140,
      })
  })
  return result
}

export function Topology({
  nodes,
  edges,
  declared,
  selected,
  onSelect,
  onClear,
}: {
  nodes: readonly MapNode[]
  edges: readonly MapConnection[]
  declared: DeclaredTopology | undefined
  selected: string | null
  onSelect: (id: string) => void
  onClear: () => void
}) {
  const [zoom, setZoom] = useState(1)
  const viewport = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ width: 900, height: 600 })
  useEffect(() => {
    if (!viewport.current) return
    const observer = new ResizeObserver(([entry]) => {
      if (entry)
        setSize({
          width: Math.max(280, entry.contentRect.width),
          height: Math.max(300, entry.contentRect.height),
        })
    })
    observer.observe(viewport.current)
    return () => observer.disconnect()
  }, [])
  const width = size.width
  const slots = positions(
    nodes.map((node) => node.id).sort(),
    width,
    size.height
  )
  const height = Math.max(
    size.height,
    ...[...slots.values()].map((point) => point.y + 100)
  )
  const byId = new Map(nodes.map((node) => [node.id, node]))
  const peers = new Set(
    edges.flatMap((link) =>
      link.a === selected ? [link.b] : link.b === selected ? [link.a] : []
    )
  )

  return (
    <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 opacity-40"
        style={{
          backgroundImage:
            "radial-gradient(var(--border) 1px, transparent 1px)",
          backgroundSize: "24px 24px",
          maskImage:
            "radial-gradient(ellipse at center, black, transparent 75%)",
        }}
      />
      <div className="absolute top-3 right-3 z-10 flex items-center gap-1 rounded-full bg-fill-subtle p-1">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={m.overview_zoom_out()}
          onClick={() => setZoom((value) => Math.max(0.5, value - 0.25))}
        >
          <MinusIcon />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={m.overview_fit()}
          onClick={() => {
            setZoom(1)
            viewport.current?.scrollTo(0, 0)
          }}
        >
          <ScanIcon />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={m.overview_zoom_in()}
          onClick={() => setZoom((value) => Math.min(3, value + 0.25))}
        >
          <PlusIcon />
        </Button>
      </div>
      <div
        ref={viewport}
        className="min-h-0 flex-1 overflow-auto"
        onClick={(event) => {
          if (event.target === event.currentTarget) onClear()
        }}
      >
        <svg
          viewBox={`0 0 ${width} ${height}`}
          width={width * zoom}
          height={height * zoom}
          className="relative mx-auto block"
          aria-label={m.overview_topology()}
          onClick={(event) => {
            if (event.target === event.currentTarget) onClear()
          }}
        >
          {edges.map((link) => {
            const a = slots.get(link.a),
              b = slots.get(link.b)
            if (!a || !b) return null
            const x = (a.x + b.x) / 2 + (a.x === b.x ? 70 : 0)
            const y = (a.y + b.y) / 2
            const active =
              selected === null || link.a === selected || link.b === selected
            return (
              <path
                key={link.key}
                d={`M ${a.x} ${a.y} Q ${x} ${y} ${b.x} ${b.y}`}
                fill="none"
                stroke={lookColor[link.look]}
                strokeWidth={link.look === "matched" ? 2 : 1.5}
                strokeDasharray={
                  link.look === "matched" || link.look === "history"
                    ? undefined
                    : "6 6"
                }
                opacity={
                  (link.look === "history" ? 0.25 : 0.8) * (active ? 1 : 0.2)
                }
              >
                <title>
                  {[
                    `${byId.get(link.a)?.name} ↔ ${byId.get(link.b)?.name}`,
                    mapConnectionText(link),
                    link.transports.map(transportText).join(" / "),
                    link.declaration
                      ? link.declaration.rule === null
                        ? m.overview_topology_default()
                        : m.overview_topology_rule({
                            number: link.declaration.rule + 1,
                          })
                      : null,
                    link.declaration?.transports?.length
                      ? m.overview_allowed_transports({
                          value: link.declaration.transports
                            .map(transportText)
                            .join(" / "),
                        })
                      : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </title>
              </path>
            )
          })}
          {nodes.map((node) => {
            const point = slots.get(node.id)
            if (!point) return null
            return (
              <foreignObject
                key={node.id}
                x={point.x - 100}
                y={point.y - 26}
                width={200}
                height={108}
                style={{
                  opacity:
                    selected === null ||
                    selected === node.id ||
                    peers.has(node.id)
                      ? 1
                      : 0.5,
                }}
              >
                <button
                  type="button"
                  aria-pressed={selected === node.id}
                  onClick={() => onSelect(node.id)}
                  title={`${node.name} · ${statusLabel[node.status]()}`}
                  className="group flex w-full flex-col items-center gap-3 rounded-row p-1 text-center outline-none"
                >
                  <span className="relative flex size-12 items-center justify-center rounded-2xl border border-border bg-background shadow-sm transition-colors group-hover:border-foreground/30 group-focus-visible:ring-2 group-focus-visible:ring-ring group-aria-pressed:border-foreground/50 group-aria-pressed:bg-fill-selected">
                    <ServerIcon
                      className="size-5 text-muted-foreground group-aria-pressed:text-foreground"
                      strokeWidth={1.5}
                    />
                    <span className="absolute -right-1 -bottom-1 flex size-4 items-center justify-center rounded-full bg-background">
                      <NodeStatusDot status={node.status} />
                    </span>
                  </span>
                  <span className="max-w-full truncate text-row font-medium">
                    {node.name}
                  </span>
                </button>
              </foreignObject>
            )
          })}
        </svg>
      </div>
      {nodes.length > 0 && edges.length === 0 ? (
        <p className="pointer-events-none absolute bottom-4 left-4 text-meta text-muted-foreground">
          {declared ? m.overview_topology_empty() : m.nodes_links_none()}
        </p>
      ) : null}
    </div>
  )
}
