import {
  useLocation,
  useNavigate,
  useRouter,
  type HistoryState,
} from "@tanstack/react-router"
import { useMemo } from "react"

declare module "@tanstack/react-router" {
  interface HistoryState {
    /** On an entry pushed to open a peek, a workspace or a page sheet: closing it goes back. */
    opened?: true
    /**
     * On phones, from the moment a page sheet opens over the overview: the
     * history index of that overview entry. However many pages and
     * workspaces follow, leaving the page returns there in one step.
     */
    pageBase?: number
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
 * - on phones, a page opened from the launcher remembers the overview it
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

  return useMemo(() => {
    const toPage = (fromOverview: boolean) =>
      fromOverview
        ? { replace: false, state: { opened: true as const, pageBase: index } }
        : { replace: true, state: { opened: true as const, pageBase } }
    return {
      /** Options for opening something: push from nothing, replace when switching. */
      open: (alreadyOpen: boolean) => ({
        replace: alreadyOpen,
        state: { opened: true as const, pageBase },
      }),
      /** Options for a change inside what is open. */
      keep: {
        replace: true,
        state: (previous: HistoryState) => ({
          opened: previous.opened,
          pageBase: previous.pageBase,
        }),
      },
      /** Options for the launcher's move to another page: from the overview it pushes and remembers it; between pages it replaces. */
      toPage,
      /** Closes what is open: back where it was opened from, or else `fallback` (which should replace). */
      close(fallback: () => void) {
        if (opened) router.history.back()
        else fallback()
      },
      /** Leaves the pages for the overview: back to the overview they were opened from, wherever that is. */
      leavePage() {
        if (pageBase !== undefined && pageBase < index)
          router.history.go(pageBase - index)
        else if (opened) router.history.back()
        else
          void navigate({
            to: "/networks/$network/overview",
            params: { network: networkId },
            replace: true,
          })
      },
    }
  }, [router, navigate, opened, pageBase, index, networkId])
}
