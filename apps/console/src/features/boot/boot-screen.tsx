import { useRouterState } from "@tanstack/react-router"
import { useEffect } from "react"

import { m } from "@/paraglide/messages.js"

import { useBootStore } from "./boot-store"
import { LoaderGlobe } from "./loader-globe"

/**
 * The held loading screen of the first load. It covers everything until the
 * globe renderer and the first route's data are ready. Then its backdrop
 * goes at once (the globe stage draws the very same one underneath) and only
 * the loader globe cross-fades into the real globe, which is already in the
 * same place and pose and starts growing straight away.
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
      className="fixed inset-0 z-50 data-[phase=revealing]:pointer-events-none"
    >
      {phase === "loading" ? <div className="globe-backdrop absolute inset-0" /> : null}
      <LoaderGlobe visible={phase === "loading"} />
    </div>
  )
}
