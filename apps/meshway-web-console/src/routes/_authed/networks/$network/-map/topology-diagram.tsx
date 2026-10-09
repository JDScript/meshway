import { Button } from "@meshway/ui/button"
import { MinusIcon, PlusIcon, ScanIcon, ServerIcon } from "lucide-react"
import { useEffect, useRef, useState } from "react"

import { NodeStatusDot } from "@/components/chrome/node-status"
import type { DeclaredTopology } from "@/lib/configuration/topology-view"
import type { MapConnection, MapNode } from "@/lib/network-map"
import { mapTransportText, statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import {
  TopologyEdge,
  type EdgeCaption,
  type EdgeDrawing,
  type Point,
} from "./topology-edge"

type Box = { left: number; right: number; top: number; bottom: number }
const overlaps = (a: Box, b: Box) =>
  a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top

/** Fit compact captions along their own curves. Dense views label incident or
 * pointed edges; others retain the full, focusable edge description.
 */
function captions(
  drawings: readonly EdgeDrawing[],
  nodes: Iterable<Point>,
  selected: string | null,
  highlighted: string | null,
  width: number,
  height: number
) {
  const result = new Map<string, EdgeCaption>()
  const occupied: Box[] = [...nodes].flatMap((point) => [
    {
      left: point.x - 29,
      right: point.x + 29,
      top: point.y - 29,
      bottom: point.y + 29,
    },
    {
      left: point.x - 100,
      right: point.x + 100,
      top: point.y + 35,
      bottom: point.y + 64,
    },
  ])
  const candidates = drawings
    .filter(
      ({ edge }) =>
        drawings.length <= 12 ||
        edge.key === highlighted ||
        edge.a === selected ||
        edge.b === selected
    )
    .sort(
      (a, b) =>
        Number(b.edge.key === highlighted) - Number(a.edge.key === highlighted)
    )
  for (const { edge, a, b, control } of candidates) {
    const text = mapTransportText(edge)
    const textWidth = Math.min(
      168,
      28 +
        [...text].reduce(
          (sum, character) =>
            sum + ((character.codePointAt(0) ?? 0) > 127 ? 12 : 6.5),
          0
        )
    )
    for (const t of [0.5, 0.62, 0.38, 0.74, 0.26, 0.86, 0.14]) {
      const point = {
        x: (1 - t) ** 2 * a.x + 2 * t * (1 - t) * control.x + t * t * b.x,
        y: (1 - t) ** 2 * a.y + 2 * t * (1 - t) * control.y + t * t * b.y,
      }
      const box = {
        left: point.x - textWidth / 2 - 3,
        right: point.x + textWidth / 2 + 3,
        top: point.y - 14,
        bottom: point.y + 14,
      }
      if (
        box.left < 4 ||
        box.right > width - 4 ||
        box.top < 4 ||
        box.bottom > height - 4 ||
        occupied.some((other) => overlaps(box, other))
      )
        continue
      result.set(edge.key, { ...point, width: textWidth, text })
      occupied.push(box)
      break
    }
    if (edge.key === highlighted && !result.has(edge.key))
      result.set(edge.key, {
        x: (a.x + 2 * control.x + b.x) / 4,
        y: (a.y + 2 * control.y + b.y) / 4,
        width: textWidth,
        text,
      })
  }
  return result
}

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

export function TopologyDiagram({
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
  const [pointedEdge, pointAtEdge] = useState<string | null>(null)
  const [focusedEdge, focusEdge] = useState<string | null>(null)
  const highlighted = pointedEdge ?? focusedEdge
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
  const drawings: EdgeDrawing[] = edges.flatMap((edge) => {
    const a = slots.get(edge.a),
      b = slots.get(edge.b)
    if (!a || !b) return []
    return [
      {
        edge,
        a,
        b,
        control: {
          x:
            (a.x + b.x) / 2 +
            (a.x === b.x
              ? Math.min(Math.abs(a.y - b.y) * 0.65, width / 2 - 24)
              : 0),
          y: (a.y + b.y) / 2,
        },
      },
    ]
  })
  const labels = captions(
    drawings,
    slots.values(),
    selected,
    highlighted,
    width,
    height
  )
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
          {drawings.map((drawing) => (
            <TopologyEdge
              key={drawing.edge.key}
              drawing={drawing}
              caption={labels.get(drawing.edge.key)}
              names={(id) => byId.get(id)?.name ?? id.slice(0, 8)}
              active={
                selected === null ||
                drawing.edge.a === selected ||
                drawing.edge.b === selected
              }
              highlighted={highlighted === drawing.edge.key}
              focusable={
                edges.length <= 12 ||
                drawing.edge.a === selected ||
                drawing.edge.b === selected
              }
              onPoint={pointAtEdge}
              onFocus={focusEdge}
            />
          ))}
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
