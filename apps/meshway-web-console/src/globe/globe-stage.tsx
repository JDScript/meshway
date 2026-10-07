import { useEffect, useRef, useState } from "react"

import { usePrefersReducedMotion } from "@/hooks/use-media-query"
import { m } from "@/paraglide/messages.js"
import {
  markEngineReady,
  markEngineUnavailable,
  markRevealed,
  useBootPhase,
} from "@/state/boot"
import { pointAtOnGlobe, selectFromGlobe, useGlobe } from "@/state/globe"

import { readGlobeColors, type GlobeColors } from "./colors"
import type { GlobeEngine } from "./engine"

/** Theme colours, re-read whenever the root's theme class changes. */
function useGlobeColors(): GlobeColors {
  const [colors, setColors] = useState(readGlobeColors)
  useEffect(() => {
    const observer = new MutationObserver(() => setColors(readGlobeColors()))
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style"],
    })
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
  const boot = useBootPhase()
  const colors = useGlobeColors()
  const motion = !usePrefersReducedMotion()

  useEffect(() => {
    const element = host.current
    if (!element) return
    let disposed = false
    let created: GlobeEngine | null = null
    let observer: ResizeObserver | null = null
    import("./engine")
      .then(({ createGlobeEngine }) => {
        if (disposed) return
        created = createGlobeEngine(
          element,
          selectFromGlobe,
          pointAtOnGlobe,
          markRevealed
        )
        const instance = created
        observer = new ResizeObserver(([entry]) => {
          if (entry)
            instance.resize(entry.contentRect.width, entry.contentRect.height)
        })
        observer.observe(element)
        instance.resize(element.clientWidth, element.clientHeight)
        setEngine(instance)
      })
      // No WebGL, or the renderer's files did not load: no globe, the rest goes on.
      .catch(() => {
        if (!disposed) markEngineUnavailable()
      })
    return () => {
      disposed = true
      observer?.disconnect()
      created?.destroy()
      setEngine(null)
    }
  }, [])

  useEffect(() => {
    if (!engine) return
    engine.update({ ...state, colors, motion, boot })
  }, [engine, state, colors, motion, boot])

  // Ready once the renderer has drawn the loader's pose at least once, so
  // the hand-off never uncovers an empty canvas.
  useEffect(() => {
    if (!engine) return
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(markEngineReady)
    })
    return () => cancelAnimationFrame(frame)
  }, [engine])

  return (
    <>
      <div className="globe-backdrop fixed inset-0" />
      <div
        ref={host}
        role="region"
        aria-label={m.globe_region()}
        data-boot={boot}
        data-receded={state.recede}
        className="globe-stage fixed inset-0"
      />
    </>
  )
}
