import { useEffect, useRef, useState, useSyncExternalStore } from "react"

import { useBootStore } from "@/features/boot/boot-store"
import { m } from "@/paraglide/messages.js"

import { readGlobeColors, type GlobeColors } from "./colors"
import type { GlobeEngine } from "./engine"
import { selectFromGlobe, useGlobeStore } from "./store"

const REDUCED_MOTION = "(prefers-reduced-motion: reduce)"

function usePrefersMotion(): boolean {
  return useSyncExternalStore(
    (onChange) => {
      const query = window.matchMedia(REDUCED_MOTION)
      query.addEventListener("change", onChange)
      return () => query.removeEventListener("change", onChange)
    },
    () => !window.matchMedia(REDUCED_MOTION).matches
  )
}

/** Theme colours, re-read whenever the root's theme class changes. */
function useGlobeColors(): GlobeColors {
  const [colors, setColors] = useState(readGlobeColors)
  useEffect(() => {
    const observer = new MutationObserver(() => setColors(readGlobeColors()))
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style", "data-lab"] })
    return () => observer.disconnect()
  }, [])
  return colors
}

/**
 * The one globe of the console. It lives above every route, so navigating
 * never re-creates the WebGL context; routes steer it through the store.
 */
export function GlobeStage() {
  const host = useRef<HTMLDivElement>(null)
  const [engine, setEngine] = useState<GlobeEngine | null>(null)
  const state = useGlobeStore()
  const boot = useBootStore((store) => store.phase)
  const markEngineReady = useBootStore((store) => store.markEngineReady)
  const markRevealed = useBootStore((store) => store.markRevealed)
  const colors = useGlobeColors()
  const motion = usePrefersMotion()

  useEffect(() => {
    const element = host.current
    if (!element) return
    let disposed = false
    let created: GlobeEngine | null = null
    let observer: ResizeObserver | null = null
    void import("./engine").then(({ createGlobeEngine }) => {
      if (disposed) return
      created = createGlobeEngine(element, selectFromGlobe, markRevealed)
      const instance = created
      observer = new ResizeObserver(([entry]) => {
        if (entry) instance.resize(entry.contentRect.width, entry.contentRect.height)
      })
      observer.observe(element)
      instance.resize(element.clientWidth, element.clientHeight)
      setEngine(instance)
    })
    return () => {
      disposed = true
      observer?.disconnect()
      created?.destroy()
      setEngine(null)
    }
  }, [markRevealed])

  useEffect(() => {
    if (!engine) return
    engine.update({ ...state, colors, motion, boot })
    markEngineReady()
  }, [engine, state, colors, motion, boot, markEngineReady])

  return (
    <div className="globe-layer globe-backdrop fixed inset-0">
      <div ref={host} role="region" aria-label={m.globe_region()} data-boot={boot} className="globe-stage size-full" />
    </div>
  )
}
