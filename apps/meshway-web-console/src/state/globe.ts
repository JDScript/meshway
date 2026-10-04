import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import { EMPTY_SCENE, type Scene } from "@/features/globe/scene"

// The globe is created once, above every route, and never re-created on
// navigation. Routes describe what it should show through this store; what
// a click on a node means is the network layout's (`onGlobeSelect`), held
// as a runtime handler, not state.

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
  /** The node the pointer is on, on the globe (the stage reports it). */
  pointedNodeId: string | null
  /** A node pointed at elsewhere (a list row); highlighted, the camera stays. */
  hoveredNodeId: string | null
  insets: Insets
  /** A workspace covers the globe: it dims and stays where it is. */
  recede: boolean
}

const globeStore = createStore<GlobeState>(() => ({
  framing: "welcome",
  scene: EMPTY_SCENE,
  selectedNodeId: null,
  pointedNodeId: null,
  hoveredNodeId: null,
  insets: NO_INSETS,
  recede: false,
}))

/** What the globe shows, for its stage. */
export const useGlobe = (): GlobeState => useStore(globeStore)

/** The node the pointer is on, on the globe; null when none. */
export const usePointedNode = (): string | null =>
  useStore(globeStore, (globe) => globe.pointedNodeId)

export function setGlobe(patch: Partial<GlobeState>) {
  const current = globeStore.getState()
  // Panels re-measure often; equal insets are not a change.
  if (patch.insets && sameInsets(patch.insets, current.insets))
    patch = { ...patch, insets: current.insets }
  globeStore.setState(patch)
}

/** The empty globe beside the sign-in card or a status card. */
export function showWelcomeGlobe() {
  setGlobe({
    framing: "welcome",
    scene: EMPTY_SCENE,
    selectedNodeId: null,
    pointedNodeId: null,
    hoveredNodeId: null,
    insets: NO_INSETS,
    recede: false,
  })
}

/** Lists point at a node on the globe (or stop pointing). The shell writes everything else. */
export function highlightOnGlobe(nodeId: string | null) {
  if (globeStore.getState().hoveredNodeId !== nodeId)
    globeStore.setState({ hoveredNodeId: nodeId })
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

/** The stage reports where the pointer is. */
export function pointAtOnGlobe(nodeId: string | null) {
  if (globeStore.getState().pointedNodeId !== nodeId)
    globeStore.setState({ pointedNodeId: nodeId })
}
