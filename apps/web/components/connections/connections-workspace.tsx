"use client"

import type { ConnectionsSnapshot } from "@meshway/control-client"

import { useState } from "react"

import { ConnectionsEditor } from "@/components/connections/connections-editor"
import { useConnectionRuntimes, type InitialRuntimes } from "@/components/connections/use-runtimes"

/** Network-wide connections: overview, directed dials and listeners. */
export function ConnectionsWorkspace({
  snapshot,
  initialRuntimes,
  managedListenerIds,
}: {
  snapshot: ConnectionsSnapshot
  initialRuntimes: InitialRuntimes
  /** Platform-managed listener IDs, or null when the managed read failed. */
  managedListenerIds: string[] | null
}) {
  const runtimes = useConnectionRuntimes(initialRuntimes)
  const [managed] = useState(() => (managedListenerIds === null ? null : new Set(managedListenerIds)))
  return <ConnectionsEditor snapshot={snapshot} managedListenerIds={managed} scopeNodeId={null} runtimes={runtimes} />
}
