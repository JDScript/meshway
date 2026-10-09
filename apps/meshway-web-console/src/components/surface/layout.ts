import type { Insets } from "@/state/globe"

// Where the floating surfaces sit, in CSS pixels. The surfaces and the
// globe both read these numbers: the globe centres itself (and shrinks if it
// must) in the space the surfaces leave free, and recedes when a workspace
// covers it.
//
//   chrome     always: network card top left, preferences top right, the dock
//              (phones: preferences top right; the launcher and the card at the bottom)
//   page       one at a time, left of the free space (a list)
//   workspace  one entity in full, covering the globe right of the page panel

export const EDGE = 20
export const BAR_HEIGHT = 44
export const GAP = 12
/** Surfaces start below the top bar. */
export const PANEL_TOP = EDGE + BAR_HEIGHT + GAP
export const DOCK_WIDTH = 48
export const PAGE_LEFT = EDGE + DOCK_WIDTH + GAP
export const PAGE_WIDTH = 340
export const WORKSPACE_LEFT = PAGE_LEFT + PAGE_WIDTH + GAP

/** On small screens one inset for everything: the top bar, the bottom bar and the sheets. */
export const MOBILE_EDGE = 12
/** Below the round buttons of the top bar. */
export const MOBILE_PANEL_TOP = MOBILE_EDGE + BAR_HEIGHT + GAP
/**
 * The bottom bar (the page launcher, where the thumb is, and the network
 * card) stays on every page; sheets float above it, this far from the
 * bottom of the screen.
 */
const MOBILE_BAR_SPACE = MOBILE_EDGE + BAR_HEIGHT
export const MOBILE_SHEET_BOTTOM = MOBILE_BAR_SPACE + MOBILE_EDGE
/** Share of the viewport height the bottom sheets take on small screens. */
export const MOBILE_PAGE_HEIGHT = 0.52
/** The map pages' panel on small screens: shorter, so the topology above it has room. */
export const MOBILE_MAP_SHEET = "40dvh"

/** A bottom sheet leaving through the screen's edge travels this much further, so its shadow goes too. */
export const SHEET_EXIT_OVERSHOOT = 2

/**
 * The commit bar floats this far above the bottom of a workspace; in a
 * phone's sheet it sits closer, concentric with the sheet's corners
 * (28 − 6 = 22, the bar's own radius).
 */
export const COMMIT_BOTTOM = 16
export const COMMIT_SHEET_INSET = 6

/** Room an editor keeps free at the bottom of its workspace for the commit bar floating there. */
const COMMIT_SPACE = BAR_HEIGHT + 2 * COMMIT_BOTTOM

/** The geometry above as CSS variables, for surfaces placed with classes; set once on the document. */
export const LAYOUT_VARIABLES: Record<`--${string}`, string> = {
  "--edge": `${EDGE}px`,
  "--bar-height": `${BAR_HEIGHT}px`,
  "--panel-top": `${PANEL_TOP}px`,
  "--mobile-edge": `${MOBILE_EDGE}px`,
  "--commit-space": `${COMMIT_SPACE}px`,
}

export type Arrangement = {
  mobile: boolean
  page: boolean
  workspace: boolean
  viewportHeight: number
}

/** The globe's view of the surfaces: how much of each edge they cover, and whether it is covered. */
export function globeLayout({
  mobile,
  page,
  workspace,
  viewportHeight,
}: Arrangement): { insets: Insets; recede: boolean } {
  if (mobile) {
    return {
      insets: {
        top: 0,
        right: 0,
        bottom: page
          ? Math.round(MOBILE_PAGE_HEIGHT * viewportHeight) +
            MOBILE_SHEET_BOTTOM
          : MOBILE_BAR_SPACE,
        left: 0,
      },
      recede: workspace,
    }
  }
  return {
    // Under a workspace the globe keeps the place it had beside the page panel.
    insets: {
      top: 0,
      right: 0,
      bottom: 0,
      left: page ? WORKSPACE_LEFT : 0,
    },
    recede: workspace,
  }
}
