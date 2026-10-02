import { useQuery } from "@tanstack/react-query"
import { useMemo } from "react"

import { clockTime } from "@/lib/format"
import { useNow } from "@/lib/use-now"

import { linkSummary, overviewScene } from "../overview/model"
import { connectionsQuery, networksQuery, nodesQuery, runtimesQuery } from "./queries"

/**
 * A network as the console presents it: the cached reads, the globe scene
 * derived from them and the corner summary. Every page under a network uses
 * this one view, so the globe and the summaries never disagree.
 */
export function useNetworkView(networkId: string) {
  const networks = useQuery(networksQuery())
  const nodes = useQuery(nodesQuery(networkId))
  const connections = useQuery(connectionsQuery(networkId))
  const runtimes = useQuery(runtimesQuery(networkId))
  const now = useNow()

  const inventory = useMemo(() => nodes.data?.nodes ?? [], [nodes.data])
  const byId = useMemo(() => new Map(inventory.map((node) => [node.id, node])), [inventory])
  const { scene, observation } = useMemo(
    () => overviewScene({ nodes: inventory, connections: connections.data ?? null, runtimes: runtimes.data ?? null, now }),
    [inventory, connections.data, runtimes.data, now]
  )

  return {
    network: networks.data?.networks.find((candidate) => candidate.network_id === networkId) ?? null,
    nodes,
    runtimes,
    inventory,
    byId,
    scene,
    observation,
    now,
    /** null while the inventory is unread. */
    nodeCount: nodes.data ? scene.nodes.length : null,
    links: nodes.data ? linkSummary(observation, runtimes.data !== undefined) : null,
    /** Time of the last good runtime read when the latest refresh failed. */
    stale: runtimes.isError && runtimes.dataUpdatedAt > 0 ? clockTime(runtimes.dataUpdatedAt) : null,
  }
}
