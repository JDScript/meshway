import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

// First load of the console: one held loading screen until both the globe
// renderer (a large lazy chunk) and the first route's data are in, then a
// single reveal. Server data itself stays in the TanStack Query cache, which
// route loaders fill; this store only sequences the screen.

/**
 * - `loading`: the loader globe spins; nothing else is shown.
 * - `revealing`: the real globe takes over in place and grows into its frame.
 * - `ready`: the interface is in; later navigations never return here.
 */
export type BootPhase = "loading" | "revealing" | "ready"

/** The loader stays at least this long, so a fast load does not flash. */
const MIN_LOADING_MS = 900

type Boot = {
  phase: BootPhase
  engineReady: boolean
  routesReady: boolean
  minimumElapsed: boolean
}

const bootStore = createStore<Boot>(() => ({
  phase: "loading",
  engineReady: false,
  routesReady: false,
  minimumElapsed: false,
}))

/** Records one condition of the reveal; the last one met starts it. */
function met(condition: Partial<Omit<Boot, "phase">>) {
  bootStore.setState((boot) => {
    const next = { ...boot, ...condition }
    const due =
      next.phase === "loading" &&
      next.engineReady &&
      next.routesReady &&
      next.minimumElapsed
    return due ? { ...next, phase: "revealing" } : next
  })
}

/** Starts the loader's minimum time; called once, by `main.tsx`. */
export function startBoot() {
  window.setTimeout(() => met({ minimumElapsed: true }), MIN_LOADING_MS)
}

/** The globe renderer has drawn its first frame. */
export const markEngineReady = () => met({ engineReady: true })

/** The first route's data are in. */
export const markRoutesReady = () => met({ routesReady: true })

/** The real globe has grown into its frame: the interface comes in. */
export function markRevealed() {
  if (bootStore.getState().phase === "revealing")
    bootStore.setState({ phase: "ready" })
}

export const useBootPhase = (): BootPhase =>
  useStore(bootStore, (boot) => boot.phase)

/** The interface is in (the first load is over). */
export const useBooted = (): boolean =>
  useStore(bootStore, (boot) => boot.phase === "ready")
