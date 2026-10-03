import type {
  InfiniteData,
  UseInfiniteQueryResult,
} from "@tanstack/react-query"
import { useCallback } from "react"

import { ApiFailure, problemCode } from "@/lib/api"
import { toastFailure } from "@/lib/failure"

/**
 * Reads a paged list's next page. A cursor the controller no longer takes
 * (it expires) reads the list again from its start: the account's lists
 * answer it as `invalid_request`, the network's as `invalid_cursor`.
 */
export function useLoadMore<Page>(
  list: UseInfiniteQueryResult<InfiniteData<Page>>
) {
  const { fetchNextPage, refetch } = list
  return useCallback(
    () =>
      void fetchNextPage().then((result) => {
        if (!result.isFetchNextPageError) return
        const code =
          result.error instanceof ApiFailure
            ? problemCode(result.error.failure)
            : null
        if (code === "invalid_cursor" || code === "invalid_request")
          void refetch()
        else toastFailure(result.error)
      }),
    [fetchNextPage, refetch]
  )
}
