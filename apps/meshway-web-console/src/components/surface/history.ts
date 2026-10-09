import {
  useLocation,
  useNavigate,
  useRouter,
  type HistoryState,
} from "@tanstack/react-router"
import { useMemo } from "react"

import { PAGES, type CanvasPage } from "./pages"

declare module "@tanstack/react-router" {
  interface HistoryState {
    /** On an entry pushed to open a workspace or a page sheet: closing it goes back. */
    opened?: true
    /**
     * On phones, from the moment a page sheet opens over a canvas: the
     * history index of that overview/topology entry. However many pages and
     * workspaces follow, leaving the page returns there in one step.
     */
    pageBase?: number
    /** Which base canvas a phone's page sheet was opened from. */
    pageBasePage?: CanvasPage
  }
}

/**
 * The history rules every page follows, so Back and the close buttons agree:
 *
 * - opening from nothing pushes an entry marked `opened`; switching to
 *   another entity at the same level replaces it and keeps the mark;
 * - changes inside what is open (a tab, a search, a filter) replace;
 * - closing an `opened` entry goes back, anything else is replaced by the
 *   level below;
 * - on phones, a page opened from the launcher remembers the canvas it
 *   left (`pageBase`): moving to another page replaces, and leaving the
 *   pages goes straight back to that overview.
 *
 * Pages build their own typed moves from these (see the settings route's `-navigation.ts`).
 */
export function useSurfaceHistory(networkId: string) {
  const router = useRouter()
  const navigate = useNavigate()
  const opened = useLocation({
    select: (location) => location.state.opened === true,
  })
  const pageBase = useLocation({
    select: (location) => location.state.pageBase,
  })
  const index = useLocation({
    select: (location) => location.state.__TSR_index,
  })
  const pageBasePage = useLocation({
    select: (location) => location.state.pageBasePage,
  })

  return useMemo(() => {
    const toPage = (fromCanvas: CanvasPage | null) =>
      fromCanvas
        ? {
            replace: false,
            state: {
              opened: true as const,
              pageBase: index,
              pageBasePage: fromCanvas,
            },
          }
        : {
            replace: true,
            state: { opened: true as const, pageBase, pageBasePage },
          }
    const toCanvas = (target: CanvasPage = pageBasePage ?? "overview") => {
      const base = pageBasePage ?? "overview"
      if (target === base && pageBase !== undefined && pageBase < index)
        router.history.go(pageBase - index)
      else if (target === base && opened) router.history.back()
      else
        void navigate({
          to: PAGES[target].route,
          params: { network: networkId },
          search: {},
          replace: true,
        })
    }
    return {
      /** Options for opening something: push from nothing, replace when switching. */
      open: (alreadyOpen: boolean) => ({
        replace: alreadyOpen,
        state: { opened: true as const, pageBase, pageBasePage },
      }),
      /** Options for a change inside what is open. */
      keep: {
        replace: true,
        state: (previous: HistoryState) => ({
          opened: previous.opened,
          pageBase: previous.pageBase,
          pageBasePage: previous.pageBasePage,
        }),
      },
      /** From either canvas the launcher pushes and remembers it; between sheets it replaces. */
      toPage,
      /** Closes what is open: back where it was opened from, or else `fallback` (which should replace). */
      close(fallback: () => void) {
        if (opened) router.history.back()
        else fallback()
      },
      /** Dismissing a sheet returns to its base canvas. */
      leavePage: () => toCanvas(),
      /** An explicit navigation item selects that canvas, even from another base. */
      toCanvas,
    }
  }, [router, navigate, opened, pageBase, pageBasePage, index, networkId])
}
