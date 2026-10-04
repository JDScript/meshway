import { getExecutionPublication, listNodes } from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

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

/** The network's nodes, for the names of a publication's targets. */
export const progressNodesQuery = (networkId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "nodes", "inventory"],
    queryFn: ({ signal }) =>
      authed((token) => listNodes(client, { networkId }, { token, signal })),
    staleTime: 10_000,
  })
