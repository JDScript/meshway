import { cn } from "@meshway/ui/cn"
import { useMatch } from "@tanstack/react-router"
import { useEffect, useLayoutEffect, useRef, useState } from "react"

import {
  EDGE,
  GAP,
  MOBILE_EDGE,
  MOBILE_MAP_SHEET,
  MOBILE_PANEL_TOP,
  MOBILE_SHEET_BOTTOM,
  PAGE_LEFT,
  PAGE_WIDTH,
  PANEL_TOP,
} from "@/components/surface/layout"
import { Skeleton } from "@/components/ui/skeleton"
import { useIsMobile, usePrefersReducedMotion } from "@/hooks/use-media-query"
import { statusTone } from "@/lib/nodes/status"
import { statusLabel } from "@/lib/state-labels"
import { topologyDiagram, type DiagramEdge } from "@/lib/topology-diagram"
import type { Box, Point } from "@/lib/topology-layout"
import { m } from "@/paraglide/messages.js"
import { useBooted } from "@/state/boot"
import { globeMarkerPoints, hideGlobeMarkers } from "@/state/globe"

import { useNetworkScene } from "../-shell/network-scene"
import { lookColor } from "./labels"
import { edgeCaption, edgeTitle } from "./link-text"
import { useMapSelection } from "./selection"

/** How long the nodes take between the globe and the diagram, or between layouts. */
const GLIDE_MS = 640
/** Edge decorations (arrows, captions, flow) only from this many edges down, else on what is selected. */
const DECORATED = 16
/** Every name from this many nodes down, else only the selected node's and its paths'. */
const NAMED = 24

/** From the dot's centre to where a line starts. */
const NODE_GAP = 12
const ARROW = 9

type Phase = "hidden" | "entering" | "shown" | "leaving" | "arrived"

/** A line from `a` to `b`, short of both dots (and of the arrow at `b` when directed). */
export function edgeLine(a: Point, b: Point, directed: boolean) {
  const length = Math.hypot(b.x - a.x, b.y - a.y) || 1
  const ux = (b.x - a.x) / length
  const uy = (b.y - a.y) / length
  const end = NODE_GAP + (directed ? ARROW - 1 : 0)
  return `M ${(a.x + ux * NODE_GAP).toFixed(1)} ${(a.y + uy * NODE_GAP).toFixed(1)} L ${(b.x - ux * end).toFixed(1)} ${(b.y - uy * end).toFixed(1)}`
}

function arrowAt(a: Point, b: Point) {
  const length = Math.hypot(b.x - a.x, b.y - a.y) || 1
  const ux = (b.x - a.x) / length
  const uy = (b.y - a.y) / length
  const tip = { x: b.x - ux * NODE_GAP, y: b.y - uy * NODE_GAP }
  const back = { x: tip.x - ux * ARROW, y: tip.y - uy * ARROW }
  const side = ARROW * 0.55
  return [
    tip,
    { x: back.x - uy * side, y: back.y + ux * side },
    { x: back.x + uy * side, y: back.y - ux * side },
  ]
    .map((point) => `${point.x.toFixed(1)},${point.y.toFixed(1)}`)
    .join(" ")
}

const translate = (point: Point) =>
  `translate(${point.x.toFixed(1)}px, ${point.y.toFixed(1)}px)`

const easeInOut = (t: number) =>
  t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2

/**
 * Moves the drawn nodes (and the lines between them) to their places: from
 * the globe's markers when the diagram comes in, back to them when it
 * leaves, and from where they are whenever the layout changes. Written to
 * the elements on every frame, like the globe's markers; React only renders
 * where they end.
 */
function useGlide(
  targets: ReadonlyMap<string, Point>,
  phase: Phase,
  duration: number,
  onDone: (phase: Phase) => void
) {
  const root = useRef<HTMLDivElement>(null)
  const drawn = useRef(new Map<string, Point>())
  const key = [...targets]
    .map(([id, p]) => `${id}:${p.x.toFixed(1)},${p.y.toFixed(1)}`)
    .join(";")
  const latest = useRef({ targets, onDone })
  useLayoutEffect(() => {
    latest.current = { targets, onDone }
  })

  useLayoutEffect(() => {
    const element = root.current
    if (!element || phase === "hidden" || phase === "arrived") return
    const { targets: to, onDone: done } = latest.current
    const box = element.getBoundingClientRect()
    const markers = new Map(
      [...globeMarkerPoints()].flatMap(([id, point]) =>
        point.facing
          ? [[id, { x: point.x - box.left, y: point.y - box.top }] as const]
          : []
      )
    )
    const from = new Map<string, Point>()
    const end = new Map<string, Point>()
    for (const [id, point] of to) {
      const marker = markers.get(id)
      // A node with no marker to leave from (unplaced, behind the globe)
      // appears where it belongs, and fades where it is on the way out.
      from.set(
        id,
        phase === "entering"
          ? (marker ?? point)
          : (drawn.current.get(id) ?? point)
      )
      end.set(id, phase === "leaving" && marker ? marker : point)
    }
    const nodes = [...element.querySelectorAll<HTMLElement>("[data-node]")]
    for (const node of nodes)
      node.dataset.marker = markers.has(node.dataset.node ?? "")
        ? "true"
        : "false"
    const lines = [...element.querySelectorAll<SVGPathElement>("[data-line]")]
    const apply = (points: ReadonlyMap<string, Point>) => {
      for (const node of nodes) {
        const point = points.get(node.dataset.node ?? "")
        if (point) node.style.transform = translate(point)
      }
      for (const line of lines) {
        const a = points.get(line.dataset.from ?? "")
        const b = points.get(line.dataset.to ?? "")
        if (a && b)
          line.setAttribute(
            "d",
            edgeLine(a, b, line.dataset.directed === "true")
          )
      }
      drawn.current = new Map(points)
    }
    const ids = [...end.keys()]
    const stagger = ids.length > 1 ? Math.min(180, ids.length * 18) : 0
    const total = duration + stagger
    if (duration <= 0) {
      apply(end)
      element.dataset.moving = "false"
      done(phase)
      return
    }
    element.dataset.moving = "true"
    const began = performance.now()
    let frame = 0
    const tick = (now: number) => {
      const elapsed = now - began
      const points = new Map<string, Point>()
      ids.forEach((id, index) => {
        const a = from.get(id)
        const b = end.get(id)
        if (!a || !b) return
        const delay = ids.length > 1 ? (stagger * index) / (ids.length - 1) : 0
        const t = easeInOut(
          Math.min(1, Math.max(0, (elapsed - delay) / duration))
        )
        points.set(id, { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t })
      })
      apply(points)
      if (elapsed < total) frame = requestAnimationFrame(tick)
      else {
        element.dataset.moving = "false"
        done(phase)
      }
    }
    tick(began)
    return () => cancelAnimationFrame(frame)
  }, [key, phase, duration])
  return root
}

/** Reports the element's size as laid out. */
function useBox(
  ref: React.RefObject<HTMLElement | null>,
  mounted: boolean,
  onBox: (box: Box) => void
) {
  useEffect(() => {
    const element = ref.current
    if (!element) return
    const observer = new ResizeObserver(([entry]) => {
      if (entry)
        onBox({
          width: Math.round(entry.contentRect.width),
          height: Math.round(entry.contentRect.height),
        })
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [ref, mounted, onBox])
}

/**
 * The topology, drawn over the dimmed globe. Mounted for every page of the
 * network so it can come out of the globe's markers and go back into them;
 * the topology page owns its panel.
 */
export function TopologyCanvas({ networkId }: { networkId: string }) {
  const route = useMatch({
    from: "/_authed/networks/$network/topology",
    shouldThrow: false,
    select: () => true,
  })
  const shown = useBooted() && route === true
  const reduced = usePrefersReducedMotion()
  const mobile = useIsMobile()
  const [phase, setPhase] = useState<Phase>("hidden")
  // The phase follows the page: in when it opens, out when it closes.
  if (shown && phase !== "entering" && phase !== "shown") setPhase("entering")
  if (!shown && (phase === "entering" || phase === "shown")) setPhase("leaving")

  useEffect(() => {
    if (phase === "entering") hideGlobeMarkers(true)
    if (phase === "arrived") {
      hideGlobeMarkers(false)
      const timer = setTimeout(() => setPhase("hidden"), 320)
      return () => clearTimeout(timer)
    }
  }, [phase])
  useEffect(() => () => hideGlobeMarkers(false), [])

  const mounted = phase !== "hidden"
  const [box, setBox] = useState<Box>({ width: 0, height: 0 })
  const scene = useNetworkScene()
  const selection = useMapSelection(networkId, "topology")
  const [pointed, point] = useState<string | null>(null)

  const diagram = topologyDiagram({
    nodes: scene.drawnNodes,
    edges: scene.edges,
    routes: scene.routes,
    flows: scene.flows,
    selected: shown ? selection.node : null,
    link: shown ? selection.link : null,
    box,
  })
  const targets = new Map(diagram.nodes.map((node) => [node.id, node.point]))
  const root = useGlide(targets, phase, reduced ? 0 : GLIDE_MS, (finished) => {
    if (finished === "entering") setPhase("shown")
    if (finished === "leaving") setPhase("arrived")
  })
  useBox(root, mounted, setBox)

  if (!mounted) return null
  const byId = new Map(diagram.nodes.map((node) => [node.id, node]))
  const touches = (edge: DiagramEdge, id: string | null) =>
    id !== null && (edge.a === id || edge.b === id)
  const decorated = (edge: DiagramEdge) =>
    diagram.edges.length <= DECORATED ||
    edge.selected ||
    edge.key === pointed ||
    diagram.focused ||
    touches(edge, selection.node)
  const named = new Set(
    diagram.nodes.length <= NAMED
      ? diagram.nodes.map((node) => node.id)
      : diagram.nodes
          .filter((node) => node.selected || !node.dim)
          .map((node) => node.id)
  )
  const someSelected = diagram.edges.some((edge) => edge.selected)

  return (
    <div
      ref={root}
      data-phase={phase}
      data-moving="false"
      aria-hidden={!shown}
      inert={!shown}
      className={cn(
        "topology-canvas absolute",
        shown ? "pointer-events-auto" : "pointer-events-none"
      )}
      style={{
        left: mobile ? MOBILE_EDGE : PAGE_LEFT,
        top: mobile ? MOBILE_PANEL_TOP : PANEL_TOP,
        right: mobile ? MOBILE_EDGE : EDGE + PAGE_WIDTH + GAP,
        bottom: mobile
          ? `calc(${MOBILE_MAP_SHEET} + ${MOBILE_SHEET_BOTTOM + GAP}px)`
          : EDGE,
      }}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return
        if (selection.link || selection.node) selection.back()
      }}
    >
      {scene.waitingForConfiguration ? (
        <div className="flex h-full items-center justify-center text-meta text-muted-foreground">
          {scene.configurationError ? (
            m.nodes_listeners_failed()
          ) : (
            <Skeleton className="h-5 w-40 rounded-full" />
          )}
        </div>
      ) : diagram.nodes.length === 0 ? (
        <p className="flex h-full items-center justify-center text-row text-muted-foreground">
          {m.overview_no_nodes()}
        </p>
      ) : null}

      <svg
        className="map-edges pointer-events-none absolute inset-0 size-full overflow-visible"
        aria-label={m.overview_topology()}
      >
        {diagram.edges.map((edge) => {
          const from = edge.direction?.from ?? edge.a
          const to = edge.direction?.to ?? edge.b
          const a = byId.get(from)?.point
          const b = byId.get(to)?.point
          if (!a || !b) return null
          const directed = edge.direction !== null
          const line = edgeLine(a, b, directed)
          const color = lookColor[edge.look]
          const lit =
            edge.selected ||
            edge.key === pointed ||
            (!someSelected && pointed === null) ||
            touches(edge, pointed)
          return (
            <g
              key={edge.key}
              data-look={edge.look}
              data-directed={directed}
              data-lit={lit}
              data-unauthorized={edge.unauthorized}
              className="map-edge"
              style={{ color }}
            >
              {edge.connection?.inUse ? (
                // Carrying traffic: a glow, which says nothing of the way
                // it goes; the reported paths' directions flow below.
                <path
                  data-line=""
                  data-from={from}
                  data-to={to}
                  data-directed={directed}
                  d={line}
                  className="map-edge__glow"
                />
              ) : null}
              <path
                data-line=""
                data-from={from}
                data-to={to}
                data-directed={directed}
                d={line}
                className="map-edge__line"
              />
              <path
                data-line=""
                data-from={from}
                data-to={to}
                data-directed={directed}
                d={line}
                className="map-edge__hit"
                role="button"
                tabIndex={0}
                aria-label={edgeTitle(edge, scene.nameOf)}
                aria-pressed={edge.selected}
                onPointerEnter={() => point(edge.key)}
                onPointerLeave={() => point(null)}
                onFocus={() => point(edge.key)}
                onBlur={() => point(null)}
                onClick={() => selection.selectLink(edge.a, edge.b)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault()
                    selection.selectLink(edge.a, edge.b)
                  }
                }}
              >
                <title>{edgeTitle(edge, scene.nameOf)}</title>
              </path>
              {directed ? (
                <polygon
                  points={arrowAt(a, b)}
                  className="map-edge__deco map-edge__arrow"
                />
              ) : null}
              {!reduced && decorated(edge)
                ? edge.flows.map((flow) => {
                    const start = byId.get(flow.from)?.point
                    const finish = byId.get(flow.to)?.point
                    if (!start || !finish) return null
                    const path = edgeLine(start, finish, false)
                    return [0, 1, 2].map((index) => (
                      <circle
                        key={`${flow.from}>${flow.to}:${index}`}
                        r={2.25}
                        className="map-edge__deco map-edge__flow"
                      >
                        <animateMotion
                          dur="2.4s"
                          begin={`${-index * 0.8}s`}
                          repeatCount="indefinite"
                          path={path}
                        />
                      </circle>
                    ))
                  })
                : null}
            </g>
          )
        })}
      </svg>

      {diagram.edges.map((edge) => {
        if (!decorated(edge)) return null
        const text = edgeCaption(edge)
        const a = byId.get(edge.a)?.point
        const b = byId.get(edge.b)?.point
        if (text === null || !a || !b) return null
        return (
          <span
            key={edge.key}
            aria-hidden="true"
            data-lit={edge.selected || edge.key === pointed}
            className="map-edge__deco map-chip"
            style={{
              transform: translate({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }),
            }}
          >
            {text}
          </span>
        )
      })}

      {diagram.nodes.map((node) => (
        <button
          key={node.id}
          type="button"
          data-node={node.id}
          data-tone={statusTone[node.status]}
          data-selected={node.selected}
          data-dim={node.dim}
          data-named={named.has(node.id) || pointed === node.id}
          aria-pressed={node.selected}
          aria-label={`${node.name} · ${statusLabel[node.status]()}`}
          className="map-node"
          style={{ transform: translate(node.point) }}
          onPointerEnter={() => point(node.id)}
          onPointerLeave={() => point(null)}
          onClick={() =>
            node.selected ? selection.close() : selection.selectNode(node.id)
          }
        >
          <span className="map-node__hit" />
          <span className="map-node__dot" />
          <span className="map-node__label">{node.name}</span>
        </button>
      ))}
    </div>
  )
}
