import { create } from "zustand"

import { DEFAULT_LOOK, type GlobeLook } from "./look"
import { EMPTY_SCENE, type Scene } from "./scene"

// The globe is created once, above every route, and never re-created on
// navigation. Routes describe what it should show through this store.

/**
 * - `welcome`: signed out; the globe sits to one side of the sign-in card.
 * - `network`: a network's nodes and links, centred.
 * - `dock`: the console's pages; the globe shrinks into the orb at `dock`.
 */
export type Framing = "welcome" | "network" | "dock"

export type GlobeState = {
  framing: Framing
  scene: Scene
  selectedNodeId: string | null
  /** Width (px) covered by a panel on the right; the globe centres in the rest. */
  rightInset: number
  look: GlobeLook
  /** The slot the globe docks into; tracked every frame, so it follows layout changes. */
  dock: HTMLElement | null
}

export const useGlobeStore = create<GlobeState>(() => ({
  framing: "welcome",
  scene: EMPTY_SCENE,
  selectedNodeId: null,
  rightInset: 0,
  look: DEFAULT_LOOK,
  dock: null,
}))

export function setGlobe(patch: Partial<GlobeState>) {
  useGlobeStore.setState(patch)
}

let selectHandler: (nodeId: string | null) => void = () => undefined

/** The route that shows a network owns what a click on a node means. Returns the cleanup. */
export function onGlobeSelect(handler: (nodeId: string | null) => void): () => void {
  selectHandler = handler
  return () => {
    if (selectHandler === handler) selectHandler = () => undefined
  }
}

export function selectFromGlobe(nodeId: string | null) {
  selectHandler(nodeId)
}

export type ScreenPoint = { x: number; y: number; visible: boolean }

let screenListener: ((point: ScreenPoint | null) => void) | null = null

/**
 * Per-frame screen position of the selected node, for overlays that attach
 * to it (leader lines). Updates go straight to the listener, not to React.
 */
export function onSelectedScreen(listener: (point: ScreenPoint | null) => void): () => void {
  screenListener = listener
  return () => {
    if (screenListener === listener) screenListener = null
  }
}

export function emitSelectedScreen(point: ScreenPoint | null) {
  screenListener?.(point)
}
