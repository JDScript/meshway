import { createContext, useContext } from "react"

import { WORKSPACE_POPUP_CLEARANCE } from "./layout"

// Popups opened inside a workspace (a picker in a rule) stay within its
// body: below its header (on a phone, the sheet's title) and clear of the
// commit bar floating at its bottom. The body provides itself; a popup
// outside any body keeps the same clearance from the screen's edges.

const Boundary = createContext<Element | null>(null)

/** Wraps a workspace's body; `value` is that element (a callback ref's state). */
export const PopupBoundary = Boundary.Provider

/** Where a popup opened here may go: pass both to `PopoverContent`. */
export function usePopupPlacement() {
  const boundary = useContext(Boundary)
  return {
    collisionBoundary: boundary ?? ("clipping-ancestors" as const),
    collisionPadding: WORKSPACE_POPUP_CLEARANCE,
  }
}
