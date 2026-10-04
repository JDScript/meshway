import { getConfigurationProgress } from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { pollUnlessLive } from "@/features/live/status"
import { networkKey } from "@/features/network/queries"
import { authed, client } from "@/lib/api"

/** How often a revision's deployment is read again while it is shown and not streamed. */
export const PROGRESS_MS = 3_000

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
