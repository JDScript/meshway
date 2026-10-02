import { useEffect, useRef, useState, useSyncExternalStore } from "react"

import { m } from "@/paraglide/messages.js"

import { readGlobeColors, type GlobeColors } from "./colors"
import type { GlobeEngine } from "./engine"
import { selectFromGlobe, useGlobe } from "./store"

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
  const state = useGlobe()
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
      created = createGlobeEngine(element, selectFromGlobe)
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
  }, [])

  useEffect(() => {
    engine?.update({ ...state, colors, motion })
  }, [engine, state, colors, motion])

  return (
    <div className="globe-backdrop fixed inset-0">
      <div ref={host} role="region" aria-label={m.globe_region()} className="size-full" />
    </div>
  )
}
