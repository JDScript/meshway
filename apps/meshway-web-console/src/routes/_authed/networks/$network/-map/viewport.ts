import { useCallback, useEffect, useRef, useState, type RefObject } from "react"

import type { Bounds, Box, Point } from "@/lib/topology-layout"

// Zoom and pan: a view maps layout px to the canvas's px (screen =
// layout × k + offset). The canvas sizes its cards from k.

export type View = { k: number; x: number; y: number }

export const toScreen = (p: Point, view: View): Point => ({
  x: p.x * view.k + view.x,
  y: p.y * view.k + view.y,
})

export type Limits = { min: number; max: number }

/** `view` zoomed by `factor` around `at` (canvas px), within `limits`. */
export function zoomAt(view: View, factor: number, at: Point, limits: Limits) {
  const k = Math.min(limits.max, Math.max(limits.min, view.k * factor))
  const ratio = k / view.k
  return {
    k,
    x: at.x - (at.x - view.x) * ratio,
    y: at.y - (at.y - view.y) * ratio,
  }
}

/** The view that shows `area` in `box`, no closer than `max` and no farther than `min`. */
export function fitView(
  area: Bounds,
  box: Box,
  { pad = 32, max = 1, min = 0 }: { pad?: number; max?: number; min?: number }
): View {
  const width = Math.max(1, area.right - area.left)
  const height = Math.max(1, area.bottom - area.top)
  // A box not laid out yet gets a usable view, never a zero zoom.
  const k = Math.max(
    0.05,
    Math.min(
      max,
      Math.max(
        min,
        Math.min((box.width - 2 * pad) / width, (box.height - 2 * pad) / height)
      )
    )
  )
  const cx = (area.left + area.right) / 2
  const cy = (area.top + area.bottom) / 2
  return { k, x: box.width / 2 - cx * k, y: box.height / 2 - cy * k }
}

const ease = (t: number) => 1 - (1 - t) ** 3

/**
 * A view that is null until moved (then the caller's default applies), set
 * at once by zoom and pan, or glided to a target. A new `tag` (another node,
 * another size) forgets it.
 */
export function useView(tag: string) {
  const [state, setState] = useState<{ tag: string; view: View } | null>(null)
  const latestTag = useRef(tag)
  useEffect(() => {
    latestTag.current = tag
  })
  const frame = useRef(0)
  const stop = useCallback(() => cancelAnimationFrame(frame.current), [])
  const set = useCallback(
    (next: View | null) => {
      stop()
      setState(next ? { tag: latestTag.current, view: next } : null)
    },
    [stop]
  )
  const glide = useCallback(
    (from: View, to: View, ms: number) => {
      stop()
      if (ms <= 0) {
        setState({ tag: latestTag.current, view: to })
        return
      }
      const began = performance.now()
      const tick = (time: number) => {
        const t = Math.min(1, (time - began) / ms)
        const e = ease(t)
        // Zoom is eased in log space, so it does not rush at one end.
        const k = from.k * (to.k / from.k) ** e
        const fromCentre = { x: -from.x / from.k, y: -from.y / from.k }
        const toCentre = { x: -to.x / to.k, y: -to.y / to.k }
        setState({
          tag: latestTag.current,
          view: {
            k,
            x: -(fromCentre.x + (toCentre.x - fromCentre.x) * e) * k,
            y: -(fromCentre.y + (toCentre.y - fromCentre.y) * e) * k,
          },
        })
        if (t < 1) frame.current = requestAnimationFrame(tick)
      }
      frame.current = requestAnimationFrame(tick)
    },
    [stop]
  )
  useEffect(() => stop, [stop])
  return { view: state && state.tag === tag ? state.view : null, set, glide }
}

/** Not panned when pressed on these: they take their own clicks. */
const OWN_CLICKS = "[data-node],[data-cluster],.map-edge__hit,button,a"

/**
 * Wheel and pinch zoom, drag to pan, on `element`. A press that moved is a
 * pan, not a click: `wasDrag` says so to the click that follows it.
 */
export function usePanZoom(
  element: RefObject<HTMLElement | null>,
  enabled: boolean,
  on: {
    zoom: (factor: number, at: Point) => void
    pan: (dx: number, dy: number) => void
  }
) {
  const latest = useRef(on)
  useEffect(() => {
    latest.current = on
  })
  const dragged = useRef(false)
  useEffect(() => {
    const target = element.current
    if (!target || !enabled) return
    const local = (event: { clientX: number; clientY: number }) => {
      const rect = target.getBoundingClientRect()
      return { x: event.clientX - rect.left, y: event.clientY - rect.top }
    }
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      const lines = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 400 : 1
      const dx = event.deltaX * lines
      const dy = event.deltaY * lines
      // A pinch on a trackpad comes as a wheel with Ctrl: zoom.
      if (event.ctrlKey) {
        latest.current.zoom(Math.exp(-dy * 0.01), local(event))
        return
      }
      // A mouse wheel turns in whole notches: zoom, a step at a time.
      const notch =
        event.deltaMode !== 0 ||
        (dx === 0 && Math.abs(dy) >= 40 && Number.isInteger(dy))
      if (notch) latest.current.zoom(Math.exp(-dy * 0.0022), local(event))
      // Two fingers on a trackpad: pan.
      else latest.current.pan(-dx, -dy)
    }
    const pointers = new Map<number, Point>()
    let moved = 0
    let pinch: number | null = null
    const onDown = (event: PointerEvent) => {
      if (event.target instanceof Element && event.target.closest(OWN_CLICKS))
        return
      pointers.set(event.pointerId, local(event))
      target.setPointerCapture(event.pointerId)
      moved = 0
      dragged.current = false
    }
    const onMove = (event: PointerEvent) => {
      const last = pointers.get(event.pointerId)
      if (!last) return
      const now = local(event)
      pointers.set(event.pointerId, now)
      if (pointers.size === 2) {
        const [a, b] = [...pointers.values()]
        if (!a || !b) return
        const distance = Math.hypot(a.x - b.x, a.y - b.y)
        if (pinch !== null && pinch > 0)
          latest.current.zoom(distance / pinch, {
            x: (a.x + b.x) / 2,
            y: (a.y + b.y) / 2,
          })
        pinch = distance
        dragged.current = true
        return
      }
      const dx = now.x - last.x
      const dy = now.y - last.y
      moved += Math.abs(dx) + Math.abs(dy)
      if (moved > 4) {
        dragged.current = true
        latest.current.pan(dx, dy)
      }
    }
    const onUp = (event: PointerEvent) => {
      pointers.delete(event.pointerId)
      if (pointers.size < 2) pinch = null
    }
    target.addEventListener("wheel", onWheel, { passive: false })
    target.addEventListener("pointerdown", onDown)
    target.addEventListener("pointermove", onMove)
    target.addEventListener("pointerup", onUp)
    target.addEventListener("pointercancel", onUp)
    return () => {
      target.removeEventListener("wheel", onWheel)
      target.removeEventListener("pointerdown", onDown)
      target.removeEventListener("pointermove", onMove)
      target.removeEventListener("pointerup", onUp)
      target.removeEventListener("pointercancel", onUp)
    }
  }, [element, enabled])
  return { wasDrag: () => dragged.current }
}
