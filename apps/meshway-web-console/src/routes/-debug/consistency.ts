import {
  getConfigurationProgress,
  getNetwork,
  getNetworkState,
  listNodes,
} from "@meshway/control-client"
import type { QueryClient } from "@tanstack/react-query"

import { ApiFailure, authed, client, problemCode } from "@/api/client"
import { configurationProgressQuery } from "@/api/queries/configuration-progress"
import { networkKey, networkQuery } from "@/api/queries/network"
import { nodesQuery } from "@/api/queries/nodes"
import { networkStateQuery } from "@/api/queries/state"
import { differences, type Differences } from "@/lib/debug/diff"

// The reads the event stream writes, each against a fresh GET of the same
// resource that is compared and dropped (it never enters the cache): does
// "SSE first, GET as the fallback" hold the same data?

export type Resource =
  | { type: "network" }
  | { type: "nodes" }
  | { type: "state" }
  | { type: "progress"; revision: number }

export type Outcome =
  | { type: "same" }
  | { type: "different"; differences: Differences }
  | { type: "uncached" }
  | { type: "failed"; code: string | null }

export type Check = {
  resource: Resource
  outcome: Outcome
  /** State only: the projection's sequence in the cache and in the read. */
  sequences: { cached: number; read: number } | null
}

/** `cached` against what `read` returns now, and that read (null when there was none). */
async function compare<T>(
  cached: T | undefined,
  read: () => Promise<T>
): Promise<{ outcome: Outcome; read: T | null }> {
  if (cached === undefined) return { outcome: { type: "uncached" }, read: null }
  try {
    const fresh = await read()
    const found = differences(cached, fresh)
    return {
      outcome:
        found.total === 0
          ? { type: "same" }
          : { type: "different", differences: found },
      read: fresh,
    }
  } catch (error) {
    if (!(error instanceof ApiFailure)) throw error
    return {
      outcome: { type: "failed", code: problemCode(error.failure) },
      read: null,
    }
  }
}

/** Compares each read the stream writes for `networkId` with a fresh GET. */
export async function checkConsistency(
  queryClient: QueryClient,
  networkId: string
): Promise<Check[]> {
  const path = { networkId }
  const checks: Check[] = []

  const summary = queryClient.getQueryData(networkQuery(networkId).queryKey)
  const network = await compare(summary?.network, async () => {
    const read = await authed((token) => getNetwork(client, path, { token }))
    return read.network
  })
  checks.push({
    resource: { type: "network" },
    outcome: network.outcome,
    sequences: null,
  })

  const nodes = await compare(
    queryClient.getQueryData(nodesQuery(networkId).queryKey),
    () => authed((token) => listNodes(client, path, { token }))
  )
  checks.push({
    resource: { type: "nodes" },
    outcome: nodes.outcome,
    sequences: null,
  })

  const cachedState = queryClient.getQueryData(
    networkStateQuery(networkId).queryKey
  )
  const state = await compare(cachedState, () =>
    authed((token) => getNetworkState(client, path, { token }))
  )
  checks.push({
    resource: { type: "state" },
    outcome: state.outcome,
    sequences:
      cachedState && state.read
        ? { cached: cachedState.sequence, read: state.read.sequence }
        : null,
  })

  const progress = queryClient
    .getQueryCache()
    .findAll({ queryKey: [...networkKey(networkId), "progress"] })
  for (const { queryKey } of progress) {
    const revision = queryKey[3]
    if (typeof revision !== "number") continue
    const compared = await compare(
      queryClient.getQueryData(
        configurationProgressQuery(networkId, revision).queryKey
      ),
      () =>
        authed((token) =>
          getConfigurationProgress(client, { networkId, revision }, { token })
        )
    )
    checks.push({
      resource: { type: "progress", revision },
      outcome: compared.outcome,
      sequences: null,
    })
  }
  return checks
}
