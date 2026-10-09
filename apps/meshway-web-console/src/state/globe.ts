import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import { EMPTY_SCENE, type Scene } from "@/globe/scene"

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

/**
 * A page that cannot be shown: the globe comes to the middle above its card
 * and stands still, its land dim, rings pulsing out from it, amber for a
 * page not found, red for one that failed. It hides the network's markers.
 */
export type GlobeAlert = {
  tone: "missing" | "failed"
  /** CSS pixels the card covers along the bottom. */
  bottom: number
}

export type GlobeState = {
  framing: Framing
  scene: Scene
  selectedNodeId: string | null
  /** The nodes the pointer is on, on the globe: one, or a group drawn as one (the stage reports them); empty when none. */
  pointedNodeIds: readonly string[]
  /** A node pointed at elsewhere (a list row); highlighted, the camera stays. */
  hoveredNodeId: string | null
  insets: Insets
  /** A workspace covers the globe: it dims and stays where it is. */
  recede: boolean
  alert: GlobeAlert | null
  /** The topology is drawn over the globe: it stays behind, dim and out of reach. */
  backdrop: boolean
  /**
   * The topology's nodes have taken over from the markers (they leave from
   * and return to them): markers and links are not drawn meanwhile.
   */
  markersHidden: boolean
}

const globeStore = createStore<GlobeState>(() => ({
  framing: "welcome",
  scene: EMPTY_SCENE,
  selectedNodeId: null,
  pointedNodeIds: [],
  hoveredNodeId: null,
  insets: NO_INSETS,
  recede: false,
  alert: null,
  backdrop: false,
  markersHidden: false,
}))

/** What the globe shows, for its stage. */
export const useGlobe = (): GlobeState => useStore(globeStore)

/** The nodes the pointer is on, on the globe; empty when none. */
export const usePointedNodes = (): readonly string[] =>
  useStore(globeStore, (globe) => globe.pointedNodeIds)

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
    pointedNodeIds: [],
    hoveredNodeId: null,
    insets: NO_INSETS,
    recede: false,
    backdrop: false,
    markersHidden: false,
  })
}

/** The topology takes the markers over, or hands them back. */
export function hideGlobeMarkers(hidden: boolean) {
  if (globeStore.getState().markersHidden !== hidden)
    globeStore.setState({ markersHidden: hidden })
}

export const useGlobeMarkersHidden = (): boolean =>
  useStore(globeStore, (globe) => globe.markersHidden)

/** Where a node's marker is on the screen (CSS px, viewport), and whether it faces the viewer. */
export type MarkerPoint = { x: number; y: number; facing: boolean }

let markerPoints: () => ReadonlyMap<string, MarkerPoint> = () => new Map()

/** The globe tells where its markers are now; the topology starts from (and returns to) them. Returns the cleanup. */
export function provideMarkerPoints(
  provider: () => ReadonlyMap<string, MarkerPoint>
): () => void {
  markerPoints = provider
  return () => {
    if (markerPoints === provider) markerPoints = () => new Map()
  }
}

/** Every placed node's marker point; empty without a globe. Nodes of a group share its point. */
export const globeMarkerPoints = () => markerPoints()

/** Lists point at a node on the globe (or stop pointing). The shell writes everything else. */
export function highlightOnGlobe(nodeId: string | null) {
  if (globeStore.getState().hoveredNodeId !== nodeId)
    globeStore.setState({ hoveredNodeId: nodeId })
}

let selectHandler: (
  nodeId: string | null,
  group?: readonly string[]
) => boolean = () => false

/** The view that shows a network owns what a click on a node means. Returns the cleanup. */
export function onGlobeSelect(
  handler: (nodeId: string | null, group?: readonly string[]) => boolean
): () => void {
  selectHandler = handler
  return () => {
    if (selectHandler === handler) selectHandler = () => false
  }
}

export function selectFromGlobe(
  nodeId: string | null,
  group?: readonly string[]
) {
  return selectHandler(nodeId, group)
}

/** The stage reports where the pointer is. */
export function pointAtOnGlobe(nodeIds: readonly string[]) {
  const current = globeStore.getState().pointedNodeIds
  if (
    current.length !== nodeIds.length ||
    current.some((id, index) => id !== nodeIds[index])
  )
    globeStore.setState({ pointedNodeIds: nodeIds })
}

/** A fallback card shows (or stops showing) its alert on the globe; equal ones are not a change. */
export function setGlobeAlert(alert: GlobeAlert | null) {
  const current = globeStore.getState().alert
  if (
    current === alert ||
    (current !== null &&
      alert !== null &&
      current.tone === alert.tone &&
      current.bottom === alert.bottom)
  )
    return
  globeStore.setState({ alert })
}
