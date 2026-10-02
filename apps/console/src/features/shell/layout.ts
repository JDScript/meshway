import type { Insets } from "../globe/store.ts"

// Where the floating surfaces sit, in CSS pixels. The surfaces and the
// globe both read these numbers: the globe centres itself (and shrinks if it
// must) in the space the surfaces leave free, and recedes when a workspace
// covers it.
//
//   chrome     always: network card top left, preferences top right, the dock
//              (phones: preferences top right; the launcher and the card at the bottom)
//   page       one at a time, left of the free space (a list)
//   peek       the overview's quick look at one node, right
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
export const PEEK_WIDTH = 380

/** On small screens one inset for everything: the top bar, the bottom bar and the sheets. */
export const MOBILE_EDGE = 12
/** Below the round buttons of the top bar. */
export const MOBILE_PANEL_TOP = MOBILE_EDGE + BAR_HEIGHT + GAP
/**
 * The bottom bar (the page launcher, where the thumb is, and the network
 * card) stays on every page; sheets float above it, this far from the
 * bottom of the screen.
 */
export const MOBILE_BAR_SPACE = MOBILE_EDGE + BAR_HEIGHT
export const MOBILE_SHEET_BOTTOM = MOBILE_BAR_SPACE + MOBILE_EDGE
/** Share of the viewport height the bottom sheets take on small screens. */
export const MOBILE_PAGE_HEIGHT = 0.52
export const MOBILE_PEEK_HEIGHT = 0.6

/**
 * Popups opened from a workspace keep clear of the screen's edges and of
 * the commit bar (on phones, the commit bar in its sheet above the bottom
 * bar, the larger of the two).
 */
export const POPUP_CLEARANCE = {
  top: EDGE,
  right: EDGE,
  bottom: MOBILE_SHEET_BOTTOM + 6 + BAR_HEIGHT + GAP,
  left: EDGE,
}

/** Room an editor keeps free at the bottom of its workspace for the commit bar floating there. */
export const COMMIT_SPACE = BAR_HEIGHT + 32

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
  peek: boolean
  workspace: boolean
  viewportHeight: number
}

/** The globe's view of the surfaces: how much of each edge they cover, and whether it is covered. */
export function globeLayout({
  mobile,
  page,
  peek,
  workspace,
  viewportHeight,
}: Arrangement): { insets: Insets; recede: boolean } {
  if (mobile) {
    const share = peek ? MOBILE_PEEK_HEIGHT : page ? MOBILE_PAGE_HEIGHT : 0
    return {
      insets: {
        top: 0,
        right: 0,
        bottom:
          share > 0
            ? Math.round(share * viewportHeight) + MOBILE_SHEET_BOTTOM
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
      right: peek ? PEEK_WIDTH + EDGE + GAP : 0,
      bottom: 0,
      left: page ? WORKSPACE_LEFT : 0,
    },
    recede: workspace,
  }
}
