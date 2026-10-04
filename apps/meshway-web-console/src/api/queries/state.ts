import { getNetworkState } from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { authed, client } from "@/api/client"
import { pollUnlessLive } from "@/state/live"

import { networkKey } from "./network"

/**
 * The network's State as the controller shows it to this account: each
 * visible node's latest report and freshness, the links and paths their
 * reports agree on. The event stream replaces it (`state.snapshot`) and
 * applies `state.delta` to it; it is read here when the stream is not live.
 */
export const networkStateQuery = (networkId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "state"],
    queryFn: ({ signal }) =>
      authed((token) =>
        getNetworkState(client, { networkId }, { token, signal })
      ),
    staleTime: 10_000,
    refetchInterval: pollUnlessLive(networkId, 15_000),
  })
