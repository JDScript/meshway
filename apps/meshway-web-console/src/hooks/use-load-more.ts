import type {
  InfiniteData,
  UseInfiniteQueryResult,
} from "@tanstack/react-query"
import { useCallback } from "react"

import { codeOf } from "@/api/client"
import { toastFailure } from "@/api/failure"

/**
 * Reads a paged list's next page. A cursor the controller no longer takes
 * (it expires: `invalid_cursor`) reads the list again from its start.
 */
export function useLoadMore<Page>(
  list: UseInfiniteQueryResult<InfiniteData<Page>>
) {
  const { fetchNextPage, refetch } = list
  return useCallback(
    () =>
      void fetchNextPage().then((result) => {
        if (!result.isFetchNextPageError) return
        if (codeOf(result.error) === "invalid_cursor") void refetch()
        else toastFailure(result.error)
      }),
    [fetchNextPage, refetch]
  )
}
