import {
  queryObservationHistory,
  type HistoryKind,
  type HistoryMode,
} from "@meshway/control-client"
import { infiniteQueryOptions } from "@tanstack/react-query"

import { authed, client } from "@/api/client"
import { iso, type HistoryWindow } from "@/lib/history/window"
import { m } from "@/paraglide/messages.js"

import { networkKey } from "./network"

/** Items per page; the controller takes 1 to 200. */
const PAGE = 200

export type HistoryFilter = {
  mode: HistoryMode
  kinds?: HistoryKind[]
  sourceNodeId?: string
  peerNodeId?: string
}

/**
 * Recorded observations in one fixed window. The controller binds each
 * cursor to the window, filter and reader, and lets it lapse after 15
 * minutes; the window never rolls on, so pages are read once (never
 * refetched in the background) and a new window is a new query.
 */
export const historyQuery = (
  networkId: string,
  window: HistoryWindow,
  filter: HistoryFilter
) =>
  infiniteQueryOptions({
    queryKey: [...networkKey(networkId), "history", window, filter],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        queryObservationHistory(
          client,
          { networkId },
          {
            mode: filter.mode,
            from: iso(window.fromMs),
            to: iso(window.toMs),
            limit: PAGE,
            ...(filter.mode === "buckets"
              ? { bucketSeconds: window.bucketSeconds }
              : {}),
            ...(filter.kinds ? { kinds: filter.kinds } : {}),
            ...(filter.sourceNodeId
              ? { sourceNodeId: filter.sourceNodeId }
              : {}),
            ...(filter.peerNodeId ? { peerNodeId: filter.peerNodeId } : {}),
            ...(pageParam === null ? {} : { cursor: pageParam }),
          },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    staleTime: Infinity,
    gcTime: 5 * 60_000,
    refetchOnWindowFocus: false,
  })

/** How many pages are read without asking; beyond, the view says what it holds and offers more. */
export const AUTO_PAGES = 5

/** Words for the history query's refusals that name what to change. */
export const historyFailureText = {
  state_limit: () => m.history_too_many(),
  invalid_cursor: () => m.history_cursor_expired(),
  invalid_request: () => m.history_invalid_window(),
}
