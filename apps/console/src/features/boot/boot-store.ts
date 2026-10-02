import { create } from "zustand"

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

type BootState = {
  phase: BootPhase
  engineReady: boolean
  routesReady: boolean
  minimumElapsed: boolean
  markEngineReady: () => void
  markRoutesReady: () => void
  markRevealed: () => void
}

export const useBootStore = create<BootState>((set, get) => {
  const advance = () => {
    const state = get()
    if (state.phase === "loading" && state.engineReady && state.routesReady && state.minimumElapsed) set({ phase: "revealing" })
  }
  window.setTimeout(() => {
    set({ minimumElapsed: true })
    advance()
  }, MIN_LOADING_MS)
  return {
    phase: "loading",
    engineReady: false,
    routesReady: false,
    minimumElapsed: false,
    markEngineReady: () => {
      set({ engineReady: true })
      advance()
    },
    markRoutesReady: () => {
      set({ routesReady: true })
      advance()
    },
    markRevealed: () => {
      if (get().phase === "revealing") set({ phase: "ready" })
    },
  }
})
