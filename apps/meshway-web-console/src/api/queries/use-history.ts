import type {
  HistoryBucket,
  HistoryCoverage,
  HistoryPage,
  RecentLink,
} from "@meshway/control-client"
import { useInfiniteQuery } from "@tanstack/react-query"
import { useEffect } from "react"

import type { HistoryWindow } from "@/lib/history/window"

import { AUTO_PAGES, historyQuery, type HistoryFilter } from "./history"

/**
 * One window's pages: the first `AUTO_PAGES` are read on their own, the
 * rest only when asked (`more`). `complete` once the controller has no
 * further page.
 */
export function useHistory(
  networkId: string,
  window: HistoryWindow | null,
  filter: HistoryFilter
) {
  const query = useInfiniteQuery({
    ...historyQuery(
      networkId,
      // Never read while off; the key is not used then.
      window ?? { choice: "1h", fromMs: 0, toMs: 0, bucketSeconds: 60 },
      filter
    ),
    enabled: window !== null,
  })
  const pages: HistoryPage[] = query.data?.pages ?? []
  const { hasNextPage, isFetchingNextPage, isError, fetchNextPage } = query
  useEffect(() => {
    if (
      hasNextPage &&
      !isFetchingNextPage &&
      !isError &&
      pages.length < AUTO_PAGES
    )
      void fetchNextPage()
  }, [hasNextPage, isFetchingNextPage, isError, pages.length, fetchNextPage])
  return {
    query,
    pages,
    /** The controller restates the window's coverage on every page; the last is the latest. */
    coverage: pages.at(-1)?.coverage ?? ([] satisfies HistoryCoverage[]),
    complete: query.data !== undefined && !hasNextPage,
    /** More pages wait to be read after the automatic ones. */
    waiting: hasNextPage && !isFetchingNextPage && pages.length >= AUTO_PAGES,
    more: () => void fetchNextPage(),
  }
}

export const bucketsOf = (pages: readonly HistoryPage[]): HistoryBucket[] =>
  pages.flatMap((page) =>
    page.result.mode === "buckets" ? page.result.items : []
  )

export const recentLinksOf = (pages: readonly HistoryPage[]): RecentLink[] =>
  pages.flatMap((page) =>
    page.result.mode === "recentLinks" ? page.result.items : []
  )

/** How many items the pages hold, whatever their mode. */
export const itemCount = (pages: readonly HistoryPage[]) =>
  pages.reduce((total, page) => total + page.result.items.length, 0)
