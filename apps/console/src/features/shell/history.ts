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
  }
}

/**
 * The history rules every page follows, so Back and the close buttons agree:
 *
 * - opening from nothing pushes an entry marked `opened`; switching to
 *   another entity at the same level replaces it and keeps the mark;
 * - changes inside what is open (a tab, a search, a filter) replace;
 * - closing an `opened` entry goes back, anything else is replaced by the
 *   level below.
 *
 * Pages build their own typed moves from these (see `features/nodes/navigation.ts`).
 */
export function useSurfaceHistory(networkId: string) {
  const router = useRouter()
  const navigate = useNavigate()
  const opened = useLocation({
    select: (location) => location.state.opened === true,
  })

  return useMemo(
    () => ({
      /** Options for opening something: push from nothing, replace when switching. */
      open: (alreadyOpen: boolean) => ({
        replace: alreadyOpen,
        state: { opened: true as const },
      }),
      /** Options for a change inside what is open. */
      keep: {
        replace: true,
        state: (previous: HistoryState) => ({ opened: previous.opened }),
      },
      /** Closes what is open: back where it was opened from, or else `fallback` (which should replace). */
      close(fallback: () => void) {
        if (opened) router.history.back()
        else fallback()
      },
      /** Leaves the page for the overview (a dismissed page sheet). */
      leavePage() {
        if (opened) router.history.back()
        else
          void navigate({
            to: "/networks/$network/overview",
            params: { network: networkId },
            replace: true,
          })
      },
    }),
    [router, navigate, opened, networkId]
  )
}

export type SurfaceHistory = ReturnType<typeof useSurfaceHistory>
