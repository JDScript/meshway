import type { AggregatedLink } from "@meshway/control-client"

export type ConnectionStatus = "connected" | "connecting" | "failed" | "unknown"

type Pair = { a: string; b: string }
export type NodePair<T> = Pair & { key: string; channels: T[] }

/** A single UI relation for a pair of nodes, independent of channel IDs and dial direction. */
export function groupNodePairs<T extends Pair>(
  records: readonly T[]
): NodePair<T>[] {
  const pairs = new Map<string, NodePair<T>>()
  for (const record of records) {
    const a = record.a < record.b ? record.a : record.b
    const b = record.a < record.b ? record.b : record.a
    const key = `${a}|${b}`
    const pair = pairs.get(key)
    if (pair) pair.channels.push(record)
    else pairs.set(key, { a, b, key, channels: [record] })
  }
  return [...pairs.values()].sort((a, b) => a.key.localeCompare(b.key))
}

export type NeighborConnection = NodePair<AggregatedLink> & {
  status: ConnectionStatus
  /** Current reported carriers. Unknown evidence does not choose an old carrier. */
  transports: string[]
}

/**
 * Direct adjacency as the visible State reports it. A current authenticated
 * report is a connection even when the caller cannot see the other end's full
 * report. Consensus remains channel evidence, not an extra product gate.
 * Expired/conflicting records do not assert a live channel or a current failure.
 */
export function neighborConnections(
  records: readonly AggregatedLink[]
): NeighborConnection[] {
  return groupNodePairs(records).flatMap((pair) => {
    const current = pair.channels.filter(
      (channel) =>
        channel.evidence === "matched" || channel.evidence === "oneSided"
    )
    const connected = current.filter(
      (channel) => channel.phase === "authenticated"
    )
    const preparing = current.filter((channel) => channel.phase === "preparing")
    const failed = current.filter((channel) => channel.phase === "failed")
    const status: ConnectionStatus = connected.length
      ? "connected"
      : preparing.length
        ? "connecting"
        : failed.length
          ? "failed"
          : "unknown"
    const basis = connected.length
      ? connected
      : preparing.length
        ? preparing
        : failed
    // Closed-only pairs leave this view. Uncertain records of another channel
    // cannot be declared closed on the strength of one channel's closure.
    if (
      pair.channels.every(
        (channel) =>
          channel.phase === "closed" && channel.evidence !== "conflicting"
      )
    )
      return []
    return [
      {
        ...pair,
        status,
        transports: [
          ...new Set(basis.map((channel) => channel.transport)),
        ].sort(),
      },
    ]
  })
}

export function nodeConnections(
  records: readonly AggregatedLink[],
  nodeId: string
) {
  return neighborConnections(records)
    .filter((connection) => connection.a === nodeId || connection.b === nodeId)
    .map((connection) => ({
      ...connection,
      peerId: connection.a === nodeId ? connection.b : connection.a,
    }))
}
