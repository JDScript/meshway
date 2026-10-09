import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import { useMatch } from "@tanstack/react-router"
import { MinusIcon, PlusIcon, ScanIcon, XIcon } from "lucide-react"
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react"

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
import { Spinner } from "@/components/ui/spinner"
import { useIsMobile, usePrefersReducedMotion } from "@/hooks/use-media-query"
import { statusTone } from "@/lib/nodes/status"
import { statusLabel } from "@/lib/state-labels"
import { topologyDiagram, type DiagramEdge } from "@/lib/topology-diagram"
import {
  bounds,
  GRAPH_CARD,
  GRAPH_PITCH,
  graphLayout,
  PATH_CARD,
  PATH_ROW,
  pathLayout,
  type Box,
  type Point,
} from "@/lib/topology-layout"
import { m } from "@/paraglide/messages.js"
import { useBooted } from "@/state/boot"
import {
  globeMarkerPoints,
  globeView,
  hideGlobeMarkers,
  liftGlobeMarkers,
  setGlobeBackdrop,
  type GlobeDisc,
} from "@/state/globe"

import { useNetworkScene } from "../-shell/network-scene"
import { lookColor } from "./labels"
import { edgeCaption, edgeTitle } from "./link-text"
import { useMapSelection } from "./selection"
import {
  fitView,
  toScreen,
  usePanZoom,
  useView,
  zoomAt,
  type Limits,
  type View,
} from "./viewport"

/** Captions and flow on every edge from this many down, else on what is pointed at or chosen. */
const DECORATED = 16
const ARROW = 9
const CHIP_WIDTH = 160

const TIMING = {
  /** The globe stops and its dots lift. */
  lift: 480,
  /** The dots fly to their cards while the globe shrinks behind. */
  fly: 900,
  /** The longest a dot waits behind the first. */
  stagger: 160,
  /** Back onto the globe. */
  land: 780,
  relayout: 620,
}
const GRAPH_ZOOM: Limits = { min: 0.3, max: 1.6 }
const PATH_ZOOM: Limits = { min: 0.4, max: 1.6 }

type Phase =
  "hidden" | "preparing" | "entering" | "shown" | "leaving" | "arrived"
type Mode = "graph" | "paths"

const translate = (point: Point) =>
  `translate(${point.x.toFixed(1)}px, ${point.y.toFixed(1)}px)`
const easeInOut = (t: number) =>
  t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2
const clamp01 = (t: number) => Math.min(1, Math.max(0, t))
const lerp = (a: Point, b: Point, t: number) => ({
  x: a.x + (b.x - a.x) * t,
  y: a.y + (b.y - a.y) * t,
})

const cardOf = (mode: Mode) => (mode === "graph" ? GRAPH_CARD : PATH_CARD)

/**
 * How large the cards are drawn at zoom `k`: with it, but no smaller than
 * readable while the room around them allows (`slack`: how much larger a
 * card can grow before it meets its closest neighbour).
 */
function cardScale(mode: Mode, k: number) {
  const card = cardOf(mode)
  const slack = mode === "graph" ? GRAPH_PITCH / card.w : PATH_ROW / card.h
  return Math.min(Math.max(k, 0.8), k * slack)
}

/** How far from a card's centre a line towards (ux, uy) leaves its border. */
const cardEdge = (mode: Mode, ux: number, uy: number, scale: number) =>
  Math.min(
    ((cardOf(mode).w / 2) * scale) / (Math.abs(ux) || 1e-6),
    ((cardOf(mode).h / 2) * scale) / (Math.abs(uy) || 1e-6)
  ) + 4

function arrowTo(tip: Point, ux: number, uy: number) {
  const back = { x: tip.x - ux * ARROW, y: tip.y - uy * ARROW }
  const side = ARROW * 0.55
  return [
    tip,
    { x: back.x - uy * side, y: back.y + ux * side },
    { x: back.x + uy * side, y: back.y - ux * side },
  ]
    .map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`)
    .join(" ")
}

/** A line between two cards: straight in the graph; level from side to side in a node's paths. */
function edgeShape(
  mode: Mode,
  a: Point,
  b: Point,
  directed: boolean,
  scale: number
) {
  if (mode === "paths") {
    const dir = b.x >= a.x ? 1 : -1
    const half = (PATH_CARD.w / 2) * scale + 3
    const p0 = { x: a.x + dir * half, y: a.y }
    const tip = { x: b.x - dir * half, y: b.y }
    const p1 = { x: tip.x - dir * (directed ? ARROW - 1 : 0), y: tip.y }
    const pull = Math.max(24, Math.abs(p1.x - p0.x) * 0.55)
    const c0 = { x: p0.x + dir * pull, y: p0.y }
    const c1 = { x: p1.x - dir * pull, y: p1.y }
    return {
      d: `M ${p0.x.toFixed(1)} ${p0.y.toFixed(1)} C ${c0.x.toFixed(1)} ${c0.y.toFixed(1)} ${c1.x.toFixed(1)} ${c1.y.toFixed(1)} ${p1.x.toFixed(1)} ${p1.y.toFixed(1)}`,
      mid: {
        x: (p0.x + 3 * c0.x + 3 * c1.x + p1.x) / 8,
        y: (p0.y + 3 * c0.y + 3 * c1.y + p1.y) / 8,
      },
      arrow: () => arrowTo(tip, dir, 0),
      room: Math.abs(tip.x - p0.x),
    }
  }
  const length = Math.hypot(b.x - a.x, b.y - a.y) || 1
  const ux = (b.x - a.x) / length
  const uy = (b.y - a.y) / length
  const gap = cardEdge(mode, ux, uy, scale)
  const p0 = { x: a.x + ux * gap, y: a.y + uy * gap }
  const tip = { x: b.x - ux * gap, y: b.y - uy * gap }
  const back = directed ? ARROW - 1 : 0
  const p1 = { x: tip.x - ux * back, y: tip.y - uy * back }
  return {
    d: `M ${p0.x.toFixed(1)} ${p0.y.toFixed(1)} L ${p1.x.toFixed(1)} ${p1.y.toFixed(1)}`,
    mid: { x: (p0.x + p1.x) / 2, y: (p0.y + p1.y) / 2 },
    arrow: () => arrowTo(tip, ux, uy),
    room: Math.hypot(tip.x - p0.x, tip.y - p0.y),
  }
}

/** Runs `step` from 0 to 1 over `ms` on animation frames; returns the cancel. */
function animate(ms: number, step: (t: number) => void, done: () => void) {
  if (ms <= 0) {
    step(1)
    done()
    return () => undefined
  }
  const began = performance.now()
  let frame = 0
  const tick = (time: number) => {
    const t = clamp01((time - began) / ms)
    step(t)
    if (t < 1) frame = requestAnimationFrame(tick)
    else done()
  }
  tick(began)
  return () => cancelAnimationFrame(frame)
}

type Latest = { screen: ReadonlyMap<string, Point>; mode: Mode; scale: number }
type Running = { t: number; apply: (t: number) => void }
type Move = { point: Point; opacity: number }

/** Moves every drawn node, and its lines, with `move`; writes the elements directly. */
function applier(
  element: HTMLElement,
  latest: { current: Latest },
  displayed: { current: Map<string, Point> },
  move: (id: string, t: number) => Move | null
) {
  const nodeEls = [...element.querySelectorAll<HTMLElement>("[data-node]")]
  const lines = [...element.querySelectorAll<SVGPathElement>("[data-line]")]
  return (t: number) => {
    const points = new Map<string, Point>()
    for (const el of nodeEls) {
      const id = el.dataset.node ?? ""
      const at = move(id, t)
      if (!at) continue
      points.set(id, at.point)
      el.style.transform = translate(at.point)
      el.style.opacity = at.opacity >= 1 ? "" : String(at.opacity)
    }
    const { mode, scale } = latest.current
    for (const line of lines) {
      const a = points.get(line.dataset.from ?? "")
      const b = points.get(line.dataset.to ?? "")
      if (a && b)
        line.setAttribute(
          "d",
          edgeShape(mode, a, b, line.dataset.directed === "true", scale).d
        )
    }
    displayed.current = points
  }
}

/** Runs a move, marking the canvas as moving (decorations wait) until it ends. */
function run(
  element: HTMLElement,
  running: { current: Running | null },
  ms: number,
  apply: (t: number) => void,
  done: () => void
) {
  element.dataset.moving = "true"
  running.current = { t: 0, apply }
  const cancel = animate(
    ms,
    (t) => {
      if (running.current) running.current.t = t
      apply(t)
    },
    () => {
      running.current = null
      element.dataset.moving = "false"
      done()
    }
  )
  return () => {
    cancel()
    running.current = null
    element.dataset.moving = "false"
  }
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

/** The cards framed with room around them; no smaller than readable, the rest a pan away. */
function frame(
  mode: Mode,
  points: ReadonlyMap<string, Point>,
  box: Box,
  start: Point | null,
  min: number
): View {
  const area = bounds(points.values())
  const card = cardOf(mode)
  const margin = { x: card.w / 2 + 24, y: card.h / 2 + 24 }
  const width = area.right - area.left + 2 * margin.x
  const height = area.bottom - area.top + 2 * margin.y
  const view = fitView(
    {
      left: area.left - margin.x,
      top: area.top - margin.y,
      right: area.right + margin.x,
      bottom: area.bottom + margin.y,
    },
    box,
    { pad: 0, max: 1, min }
  )
  // Too large to show whole: from the given card, on the left, at its height.
  if (start && width * view.k > box.width)
    view.x = margin.x * view.k - area.left * view.k
  if (start && height * view.k > box.height)
    view.y = box.height / 2 - start.y * view.k
  return view
}

/** The globe's place behind the topology: smaller, in the middle of its canvas. */
const discFor = (rect: DOMRect): GlobeDisc => ({
  x: rect.left + rect.width / 2,
  y: rect.top + rect.height / 2,
  radius: Math.min(rect.width, rect.height) * 0.26,
})

/**
 * The topology, drawn over the globe: the globe stops, its nodes lift and
 * fly out to their cards while it shrinks behind; the cards are the whole
 * network laid out by its links, or, for a chosen node, its paths in
 * columns by hop. Both zoom and pan. Mounted for every page of the network
 * so the nodes can fly back onto the globe; the topology page owns its panel.
 */
export function TopologyCanvas({ networkId }: { networkId: string }) {
  const route = useMatch({
    from: "/_authed/networks/$network/topology",
    shouldThrow: false,
    select: () => true,
  })
  const booted = useBooted()
  const shown = booted && route === true
  // Opened at its address: the globe reveals itself straight into its place
  // behind the cards, which then come in where they are (no lift).
  const opening = !booted && route === true
  const direct = useRef(route === true)
  const reduced = usePrefersReducedMotion()
  const mobile = useIsMobile()
  const [phase, setPhase] = useState<Phase>("hidden")
  const [entry, setEntry] = useState(0)
  // The phase follows the page: in when it opens, out when it closes.
  if (shown && (phase === "hidden" || phase === "arrived")) {
    setPhase("preparing")
    setEntry(entry + 1)
  }
  if (!shown && (phase === "entering" || phase === "shown")) setPhase("leaving")
  if (!shown && phase === "preparing") setPhase("arrived")

  const mounted = phase !== "hidden" || opening
  const [box, setBox] = useState<Box>({ width: 0, height: 0 })
  const root = useRef<HTMLDivElement>(null)
  useBox(root, mounted, setBox)
  const scene = useNetworkScene()
  const selection = useMapSelection(networkId, "topology")
  const [pointed, point] = useState<string | null>(null)

  const selected = shown ? selection.node : null
  // The paths from a node wait for all their plans, so the cards move once.
  const planning = selected !== null && scene.routesPending
  const diagram = topologyDiagram({
    nodes: scene.drawnNodes,
    edges: scene.edges,
    routes: scene.routes,
    flows: scene.flows,
    selected: planning ? null : selected,
    link: shown ? selection.link : null,
  })
  const mode: Mode = diagram.tree && selected !== null ? "paths" : "graph"

  const nodes = scene.drawnNodes
  const pairs = scene.edges
  const graph = useMemo(() => graphLayout(nodes, pairs), [nodes, pairs])
  const order = useMemo(
    () =>
      [...nodes]
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((node) => node.id),
    [nodes]
  )
  const paths =
    mode === "paths" && selected && diagram.tree
      ? pathLayout(selected, order, diagram.tree)
      : null

  const graphView = useView(`${box.width}x${box.height}:${entry}`)
  const pathView = useView(`${selected}:${box.width}x${box.height}`)
  const graphFit = frame("graph", graph, box, null, 0.45)
  const pathFit =
    paths && selected
      ? frame("paths", paths, box, paths.get(selected) ?? null, 0.6)
      : null
  const view: View =
    mode === "paths"
      ? (pathView.view ?? pathFit ?? graphFit)
      : (graphView.view ?? graphFit)
  const worldOf = (id: string) =>
    (mode === "paths" ? paths?.get(id) : undefined) ?? graph.get(id)
  const screen = new Map<string, Point>()
  for (const node of diagram.nodes) {
    const at = worldOf(node.id)
    if (at) screen.set(node.id, toScreen(at, view))
  }

  // ---- Moves ------------------------------------------------------------
  const scale = cardScale(mode, view.k)
  const latest = useRef<Latest>({ screen, mode, scale })
  const displayed = useRef(new Map<string, Point>())
  const running = useRef<Running | null>(null)
  useLayoutEffect(() => {
    latest.current = { screen, mode, scale }
  })

  // Out of the globe, and back onto it.
  useLayoutEffect(() => {
    const element = root.current
    if (!element || phase === "hidden") return
    if (phase === "arrived") {
      direct.current = false
      hideGlobeMarkers(false)
      liftGlobeMarkers(false)
      const timer = setTimeout(() => setPhase("hidden"), 320)
      return () => clearTimeout(timer)
    }
    if (box.width === 0 || box.height === 0) return
    const rect = element.getBoundingClientRect()
    const local = (p: Point) => ({ x: p.x - rect.left, y: p.y - rect.top })

    if (phase === "preparing") {
      if (direct.current || reduced || !globeView()) {
        // The globe goes behind at once; the cards come in where they are.
        setGlobeBackdrop("dim", discFor(rect))
        hideGlobeMarkers(true)
        element.dataset.entry = "fade"
        setPhase("shown")
        return
      }
      // The globe stops where it is; its dots lift off.
      const now = globeView()
      if (now)
        setGlobeBackdrop(null, { x: now.x, y: now.y, radius: now.radius })
      liftGlobeMarkers(true)
      const timer = setTimeout(() => setPhase("entering"), TIMING.lift)
      return () => clearTimeout(timer)
    }

    if (phase === "entering") {
      element.dataset.entry = "fly"
      const markers = globeMarkerPoints()
      const globe = globeView()
      const centre = globe
        ? local(globe)
        : { x: rect.width / 2, y: rect.height / 2 }
      setGlobeBackdrop("dim", discFor(rect))
      liftGlobeMarkers(false)
      hideGlobeMarkers(true)
      const ids = [...latest.current.screen.keys()]
      // From the middle of the globe out, one after another.
      const from = new Map<string, Point | null>()
      for (const id of ids) {
        const marker = markers.get(id)
        from.set(id, marker?.facing ? local(marker) : null)
      }
      const distance = (id: string) => {
        const p = from.get(id)
        return p ? Math.hypot(p.x - centre.x, p.y - centre.y) : Infinity
      }
      const ranked = [...ids].sort((a, b) => distance(a) - distance(b))
      const delay = new Map(
        ranked.map((id, index) => [
          id,
          ranked.length > 1
            ? (index / (ranked.length - 1)) * (TIMING.stagger / TIMING.fly)
            : 0,
        ])
      )
      const span = 1 + TIMING.stagger / TIMING.fly
      const apply = applier(element, latest, displayed, (id, t) => {
        const final = latest.current.screen.get(id)
        if (!final) return null
        const start = from.get(id)
        const local = easeInOut(clamp01(t * span - (delay.get(id) ?? 0)))
        // Not seen on the globe (behind it, or no place): it appears where it goes.
        if (!start) return { point: final, opacity: local }
        return { point: lerp(start, final, local), opacity: 1 }
      })
      return run(element, running, TIMING.fly + TIMING.stagger, apply, () =>
        setPhase("shown")
      )
    }

    if (phase === "leaving") {
      element.dataset.entry = "fly"
      // The globe goes back to its place; the dots follow their markers there.
      setGlobeBackdrop(null, null)
      const from = new Map(displayed.current)
      const apply = applier(element, latest, displayed, (id, t) => {
        const start = from.get(id)
        if (!start) return null
        const e = easeInOut(t)
        const marker = globeMarkerPoints().get(id)
        if (marker?.facing)
          return { point: lerp(start, local(marker), e), opacity: 1 }
        const globe = globeView()
        const centre = globe ? local(globe) : start
        return { point: lerp(start, centre, e * 0.6), opacity: 1 - e }
      })
      return run(element, running, reduced ? 0 : TIMING.land, apply, () => {
        setGlobeBackdrop(null, null)
        setPhase("arrived")
      })
    }
  }, [phase, reduced, box.width, box.height])
  // The globe's place behind the cards: known before its reveal aims, when
  // the page is opened at its address; kept as the canvas resizes.
  useEffect(() => {
    const element = root.current
    if (!element || box.width === 0 || box.height === 0) return
    if (opening) {
      setGlobeBackdrop(null, discFor(element.getBoundingClientRect()))
      hideGlobeMarkers(true)
    } else if (phase === "shown")
      setGlobeBackdrop("dim", discFor(element.getBoundingClientRect()))
  }, [opening, phase, box.width, box.height])
  useEffect(
    () => () => {
      hideGlobeMarkers(false)
      liftGlobeMarkers(false)
      setGlobeBackdrop(null, null)
    },
    []
  )

  // Another layout (the graph, a node's paths, nodes added): glide there.
  const layoutPoints = mode === "paths" ? (paths ?? graph) : graph
  const layoutKey = `${mode}:${selected}:${[...layoutPoints]
    .map(([id, p]) => `${id}${p.x.toFixed(0)},${p.y.toFixed(0)}`)
    .join(";")}`
  useLayoutEffect(() => {
    const element = root.current
    if (!element || phase !== "shown") return
    const from = new Map(displayed.current)
    const still = [...latest.current.screen].every(([id, at]) => {
      const was = from.get(id)
      return was && Math.abs(was.x - at.x) < 0.5 && Math.abs(was.y - at.y) < 0.5
    })
    if (from.size === 0 || still) return
    const apply = applier(element, latest, displayed, (id, t) => {
      const final = latest.current.screen.get(id)
      if (!final) return null
      return {
        point: lerp(from.get(id) ?? final, final, easeInOut(t)),
        opacity: 1,
      }
    })
    return run(
      element,
      running,
      reduced ? 0 : TIMING.relayout,
      apply,
      () => undefined
    )
    // Moves only on a new layout; zoom and pan move the cards at once.
  }, [layoutKey, phase, reduced])

  // Every commit: a move in progress stays where it is (a zoom mid-move
  // re-renders the end), and at rest the drawn places are the targets.
  useLayoutEffect(() => {
    if (running.current) running.current.apply(running.current.t)
    else displayed.current = new Map(latest.current.screen)
  })

  // ---- Zoom and pan -----------------------------------------------------
  const active = mode === "paths" ? pathView : graphView
  const limits = mode === "paths" ? PATH_ZOOM : GRAPH_ZOOM
  const { wasDrag } = usePanZoom(root, phase === "shown", {
    zoom: (factor, at) => active.set(zoomAt(view, factor, at, limits)),
    pan: (dx, dy) => active.set({ ...view, x: view.x + dx, y: view.y + dy }),
  })
  const zoomBy = (factor: number) =>
    active.glide(
      view,
      zoomAt(view, factor, { x: box.width / 2, y: box.height / 2 }, limits),
      220
    )
  const fit = () =>
    active.glide(view, mode === "paths" ? (pathFit ?? view) : graphFit, 380)

  if (!mounted) return null
  const byId = new Map(diagram.nodes.map((node) => [node.id, node]))
  const touches = (edge: DiagramEdge, id: string | null) =>
    id !== null && (edge.a === id || edge.b === id)
  const decorated = (edge: DiagramEdge) =>
    edge.selected ||
    edge.key === pointed ||
    touches(edge, pointed) ||
    touches(edge, selection.node) ||
    (mode === "graph" && diagram.edges.length <= DECORATED)
  const someSelected = diagram.edges.some((edge) => edge.selected)
  const sourceName = selected ? byId.get(selected)?.name : undefined

  return (
    <div
      ref={root}
      data-phase={phase}
      data-mode={mode}
      data-moving="false"
      aria-hidden={!shown}
      inert={!shown}
      className={cn(
        "topology-canvas absolute touch-none",
        shown ? "pointer-events-auto" : "pointer-events-none"
      )}
      style={
        {
          left: mobile ? MOBILE_EDGE : PAGE_LEFT,
          top: mobile ? MOBILE_PANEL_TOP : PANEL_TOP,
          right: mobile ? MOBILE_EDGE : EDGE + PAGE_WIDTH + GAP,
          bottom: mobile
            ? `calc(${MOBILE_MAP_SHEET} + ${MOBILE_SHEET_BOTTOM + GAP}px)`
            : EDGE,
          "--card-scale": String(scale),
        } as CSSProperties
      }
      onClick={(event) => {
        if (wasDrag()) return
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
      ) : scene.nodesLoading ? (
        <div className="flex h-full items-center justify-center">
          <Skeleton className="h-5 w-40 rounded-full" />
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
          const a = screen.get(from)
          const b = screen.get(to)
          if (!a || !b) return null
          const directed = edge.direction !== null
          const shape = edgeShape(mode, a, b, directed, scale)
          const lit =
            edge.selected ||
            edge.key === pointed ||
            (!someSelected && pointed === null) ||
            touches(edge, pointed)
          const line = (className: string) => (
            <path
              data-line=""
              data-from={from}
              data-to={to}
              data-directed={directed}
              d={shape.d}
              className={className}
            />
          )
          const title = edgeTitle(edge, scene.nameOf)
          return (
            <g
              key={edge.key}
              data-look={edge.look}
              data-directed={directed}
              data-lit={lit}
              data-unauthorized={edge.unauthorized}
              className="map-edge"
              style={{ color: lookColor[edge.look] }}
            >
              {edge.connection?.inUse ? line("map-edge__glow") : null}
              {line("map-edge__line")}
              <path
                data-line=""
                data-from={from}
                data-to={to}
                data-directed={directed}
                d={shape.d}
                className="map-edge__hit"
                role="button"
                tabIndex={0}
                aria-label={title}
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
                <title>{title}</title>
              </path>
              {directed ? (
                <polygon
                  points={shape.arrow()}
                  className="map-edge__deco map-edge__arrow"
                />
              ) : null}
              {!reduced && decorated(edge)
                ? edge.flows.map((flow) => {
                    const start = screen.get(flow.from)
                    const finish = screen.get(flow.to)
                    if (!start || !finish) return null
                    const path = edgeShape(mode, start, finish, false, scale).d
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
        const a = screen.get(edge.direction?.from ?? edge.a)
        const b = screen.get(edge.direction?.to ?? edge.b)
        if (text === null || !a || !b) return null
        const shape = edgeShape(mode, a, b, edge.direction !== null, scale)
        // Only where it fits between the cards.
        const room = Math.min(CHIP_WIDTH, shape.room - 16)
        if (room < 48) return null
        return (
          <span
            key={edge.key}
            aria-hidden="true"
            data-lit={edge.selected || edge.key === pointed}
            className="map-edge__deco map-chip"
            style={{ transform: translate(shape.mid), maxWidth: room }}
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
          data-selected={node.id === selected}
          data-dim={node.dim}
          aria-pressed={node.id === selected}
          aria-label={`${node.name} · ${statusLabel[node.status]()}`}
          className="map-node"
          style={{
            transform: translate(screen.get(node.id) ?? { x: 0, y: 0 }),
          }}
          onPointerEnter={() => point(node.id)}
          onPointerLeave={() => point(null)}
          onClick={() =>
            node.id === selected
              ? selection.close()
              : selection.selectNode(node.id)
          }
        >
          <span className="map-node__dot" />
          <span className="map-node__card">
            <span className="map-node__card-dot" />
            <span className="min-w-0 flex-1 text-left">
              <span className="block truncate text-row font-medium text-foreground">
                {node.name}
              </span>
              {mode === "paths" ? (
                <span className="block truncate text-meta text-muted-foreground">
                  {statusLabel[node.status]()}
                </span>
              ) : null}
            </span>
          </span>
        </button>
      ))}

      {mode === "paths" && sourceName ? (
        <div className="glass pointer-events-auto absolute top-0 left-0 flex h-9 items-center gap-1 rounded-full pr-1 pl-3.5 text-meta">
          <span className="font-medium">
            {m.map_paths_from({ name: sourceName })}
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            className="rounded-full"
            aria-label={m.map_paths_close()}
            onClick={() => selection.close()}
          >
            <XIcon />
          </Button>
        </div>
      ) : null}
      {planning ? (
        <span className="glass pointer-events-none absolute bottom-1 left-1/2 flex h-8 -translate-x-1/2 items-center gap-2 rounded-full px-3.5 text-meta text-muted-foreground">
          <Spinner className="size-3.5" />
          {m.map_planning()}
        </span>
      ) : null}
      {phase === "shown" ? (
        <div className="glass pointer-events-auto absolute bottom-0 left-0 flex flex-col rounded-full p-1">
          <Button
            variant="ghost"
            size="icon-sm"
            className="rounded-full"
            aria-label={m.map_zoom_in()}
            onClick={() => zoomBy(1.4)}
          >
            <PlusIcon />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            className="rounded-full"
            aria-label={m.map_zoom_out()}
            onClick={() => zoomBy(1 / 1.4)}
          >
            <MinusIcon />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            className="rounded-full"
            aria-label={m.map_fit()}
            onClick={fit}
          >
            <ScanIcon />
          </Button>
        </div>
      ) : null}
    </div>
  )
}
