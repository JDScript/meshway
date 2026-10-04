import { create } from "zustand"

import { EMPTY_SCENE, type Scene } from "./scene"

// The globe is created once, above every route, and never re-created on
// navigation. Routes describe what it should show through this store.

/**
 * - `welcome`: signed out; the globe sits to one side of the sign-in card.
 * - `network`: a network's placed nodes and their links, centred in the
 *   space the floating panels leave free.
 */
export type Framing = "welcome" | "network"

/** CSS pixels covered by floating panels along each edge. */
export type Insets = {
  top: number
  right: number
  bottom: number
  left: number
}

const NO_INSETS: Insets = { top: 0, right: 0, bottom: 0, left: 0 }

export const sameInsets = (a: Insets, b: Insets) =>
  a.top === b.top &&
  a.right === b.right &&
  a.bottom === b.bottom &&
  a.left === b.left

export type GlobeState = {
  framing: Framing
  scene: Scene
  selectedNodeId: string | null
  /** A node pointed at elsewhere (a list row); highlighted, the camera stays. */
  hoveredNodeId: string | null
  insets: Insets
  /** A workspace covers the globe: it dims and stays where it is. */
  recede: boolean
}

export const useGlobeStore = create<GlobeState>(() => ({
  framing: "welcome",
  scene: EMPTY_SCENE,
  selectedNodeId: null,
  hoveredNodeId: null,
  insets: NO_INSETS,
  recede: false,
}))

export function setGlobe(patch: Partial<GlobeState>) {
  const current = useGlobeStore.getState()
  // Panels re-measure often; equal insets are not a change.
  if (patch.insets && sameInsets(patch.insets, current.insets))
    patch = { ...patch, insets: current.insets }
  useGlobeStore.setState(patch)
}

/** The empty globe beside the sign-in card or a status card. */
export function showWelcomeGlobe() {
  setGlobe({
    framing: "welcome",
    scene: EMPTY_SCENE,
    selectedNodeId: null,
    hoveredNodeId: null,
    insets: NO_INSETS,
    recede: false,
  })
}

/** Lists point at a node on the globe (or stop pointing). The shell writes everything else. */
export function highlightOnGlobe(nodeId: string | null) {
  if (useGlobeStore.getState().hoveredNodeId !== nodeId)
    useGlobeStore.setState({ hoveredNodeId: nodeId })
}

let selectHandler: (nodeId: string | null) => void = () => undefined

/** The view that shows a network owns what a click on a node means. Returns the cleanup. */
export function onGlobeSelect(
  handler: (nodeId: string | null) => void
): () => void {
  selectHandler = handler
  return () => {
    if (selectHandler === handler) selectHandler = () => undefined
  }
}

export function selectFromGlobe(nodeId: string | null) {
  selectHandler(nodeId)
}

let hoverHandler: (nodeId: string | null) => void = () => undefined

/** The view that shows a network owns what pointing at a node on the globe shows. Returns the cleanup. */
export function onGlobeHover(
  handler: (nodeId: string | null) => void
): () => void {
  hoverHandler = handler
  return () => {
    if (hoverHandler === handler) hoverHandler = () => undefined
  }
}

export function hoverFromGlobe(nodeId: string | null) {
  hoverHandler(nodeId)
}
