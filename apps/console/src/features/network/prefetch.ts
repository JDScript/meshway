import type { QueryClient } from "@tanstack/react-query"

import {
  connectionsQuery,
  nodesQuery,
  runtimesQuery,
  sessionQuery,
} from "./queries"

/** A held screen waits for data, but never longer than this. */
const PREFETCH_LIMIT_MS = 4_000

/**
 * Fills the cache for a network's overview before the page shows. Failures
 * do not block: the page reports them itself; a slow controller is cut off.
 */
export async function prefetchOverview(
  queryClient: QueryClient,
  networkId: string
) {
  await Promise.race([
    Promise.allSettled([
      queryClient.prefetchQuery(sessionQuery()),
      queryClient.prefetchQuery(nodesQuery(networkId)),
      queryClient.prefetchQuery(connectionsQuery(networkId)),
      queryClient.prefetchQuery(runtimesQuery(networkId)),
    ]),
    new Promise((resolve) => window.setTimeout(resolve, PREFETCH_LIMIT_MS)),
  ])
}
