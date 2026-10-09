import { useMemo, useState } from "react"

import { bucketsOf, useHistory } from "@/api/queries/use-history"
import { bucketSeries } from "@/lib/history/series"
import { historyWindow } from "@/lib/history/window"

export type NeighborLatency = {
  /** Mean of the successful direct probes; null when none succeeded. */
  meanUs: number | null
  samples: number
  success: number
}

/**
 * The last hour of direct probes from a node to each peer, as the
 * controller recorded them (an observation measures; this only reads).
 * The hour is fixed when the view opens.
 */
export function useNeighborLatency(networkId: string, nodeId: string) {
  const [window] = useState(() => historyWindow("1h", Date.now()))
  const filter = useMemo(
    () => ({
      mode: "buckets" as const,
      kinds: ["probe_completed" as const],
      sourceNodeId: nodeId,
    }),
    [nodeId]
  )
  const history = useHistory(networkId, window, filter)
  const byPeer = useMemo(() => {
    const peers = new Map<string, NeighborLatency>()
    for (const series of bucketSeries(bucketsOf(history.pages), window)) {
      // Direct probes only: a neighbour probe, or a path probe over no relay.
      if (series.path.length !== 2) continue
      if (series.kind !== "neighbor" && series.kind !== "path") continue
      const kept = peers.get(series.peerNodeId) ?? {
        meanUs: null,
        samples: 0,
        success: 0,
      }
      const success = kept.success + series.totals.success
      peers.set(series.peerNodeId, {
        samples: kept.samples + series.totals.samples,
        success,
        meanUs:
          success > 0
            ? ((kept.meanUs ?? 0) * kept.success +
                (series.totals.meanUs ?? 0) * series.totals.success) /
              success
            : null,
      })
    }
    return peers
  }, [history.pages, window])
  return {
    byPeer,
    /** Not read yet: nothing is said about measurements. */
    loading: history.query.data === undefined && !history.query.isError,
    /** The read failed and nothing was read: unknown, not unmeasured. */
    failed: history.query.data === undefined && history.query.isError,
  }
}
