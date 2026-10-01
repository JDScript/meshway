"use client"

import type { ConnectionsSnapshot } from "@meshway/control-client"

import { ConnectionsEditor } from "@/components/connections/connections-editor"
import { useConnectionRuntimes, type InitialRuntimes } from "@/components/connections/use-runtimes"

/** Network-wide connections: overview, directed dials and listeners. */
export function ConnectionsWorkspace({
  snapshot,
  initialRuntimes,
}: {
  snapshot: ConnectionsSnapshot
  initialRuntimes: InitialRuntimes
}) {
  const runtimes = useConnectionRuntimes(initialRuntimes)
  return <ConnectionsEditor snapshot={snapshot} scopeNodeId={null} runtimes={runtimes} />
}
