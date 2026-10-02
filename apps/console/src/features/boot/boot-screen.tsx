import { useRouterState } from "@tanstack/react-router"
import { useEffect } from "react"

import { m } from "@/paraglide/messages.js"

import { useBootStore } from "./boot-store"
import { LoaderGlobe } from "./loader-globe"

/**
 * The held loading screen of the first load. It covers everything until the
 * globe renderer and the first route's data are ready, then hands its globe
 * over to the real one in place and fades away.
 */
export function BootScreen() {
  const phase = useBootStore((state) => state.phase)
  const markRoutesReady = useBootStore((state) => state.markRoutesReady)
  const routing = useRouterState({ select: (state) => state.status })

  useEffect(() => {
    if (routing === "idle") markRoutesReady()
  }, [routing, markRoutesReady])

  if (phase === "ready") return null
  return (
    <div
      role="status"
      aria-live="polite"
      aria-label={m.loading()}
      data-phase={phase}
      className="fixed inset-0 z-50 transition-opacity delay-150 duration-500 data-[phase=revealing]:pointer-events-none data-[phase=revealing]:opacity-0"
    >
      <div className="globe-backdrop absolute inset-0" />
      <LoaderGlobe visible={phase === "loading"} />
    </div>
  )
}
