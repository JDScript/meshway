import {
  getConfigurationProgress,
  getExecutionPublication,
} from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { pollUnlessLive } from "@/features/live/status"
import { networkKey } from "@/features/network/queries"
import { authed, client } from "@/lib/api"

import { publicationSettled } from "./progress"
import { isDenied } from "./queries"

/** How often an unsettled publication is read again while it is shown. */
const PROGRESS_MS = 3_000

/** One execution publication's per-node progress, read again until it settles. */
export const publicationQuery = (networkId: string, publicationId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "publication", publicationId],
    queryFn: ({ signal }) =>
      authed((token) =>
        getExecutionPublication(
          client,
          { networkId, publicationId },
          { token, signal }
        )
      ),
    staleTime: 0,
    refetchInterval: (query) => {
      const data = query.state.data
      if (data) return publicationSettled(data) ? false : PROGRESS_MS
      return isDenied(query.state.error) ? false : PROGRESS_MS
    },
  })

/**
 * One revision's deployment: its fixed target nodes, counted once each by
 * the controller, and each node's last accepted phase and freshness. The
 * event stream writes `configuration.progress` here; it is read here when
 * the stream is not live.
 */
export const configurationProgressQuery = (
  networkId: string,
  revision: number
) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "progress", revision],
    queryFn: ({ signal }) =>
      authed((token) =>
        getConfigurationProgress(
          client,
          { networkId, revision },
          { token, signal }
        )
      ),
    staleTime: 0,
    refetchInterval: pollUnlessLive(networkId, PROGRESS_MS),
  })
