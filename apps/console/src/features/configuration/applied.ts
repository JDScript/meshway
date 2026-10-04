import type { ConfigurationAppliedResponse } from "@meshway/control-client"
import { create } from "zustand"

import { onDropDrafts } from "@/features/shell/drafts"
import { currentSession } from "@/lib/session"

// The applies made in this tab, per sign-in, network and revision: their
// publication IDs come only from the apply's response (the controller lists
// no publications for a revision), so a revision's deployment can be
// followed only where it was applied. In memory; dropped with the sign-in.

const useApplied = create<Record<string, ConfigurationAppliedResponse>>(
  () => ({})
)
onDropDrafts(() => useApplied.setState({}, true))

/** Network IDs are UUIDs: the first "/" ends the session ID, so keys never collide. */
const keyOf = (networkId: string, revision: number) =>
  `${currentSession()?.id ?? ""}/${networkId}/${revision}`

export function recordApplied(result: ConfigurationAppliedResponse) {
  useApplied.setState({
    [keyOf(result.networkId, result.revision)]: result,
  })
}

/** This tab's apply that made `revision`, or null. */
export function useAppliedRevision(
  networkId: string,
  revision: number
): ConfigurationAppliedResponse | null {
  const key = keyOf(networkId, revision)
  return useApplied((applied) => applied[key] ?? null)
}
