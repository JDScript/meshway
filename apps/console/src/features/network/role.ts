import type { AccountRole } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"

import { sessionQuery } from "./queries"

/** This sign-in's role in the network; null until the session is read or without a membership. */
export function useNetworkRole(networkId: string): AccountRole | null {
  const session = useQuery(sessionQuery())
  return (
    session.data?.memberships.find(
      (membership) => membership.network_id === networkId
    )?.role ?? null
  )
}

/**
 * Owners and admins change a network; members read it (the controller's
 * `Manage` and `Read`). Controls a role cannot use are not offered; the
 * controller still decides every request.
 */
export function useCanManage(networkId: string): boolean {
  const role = useNetworkRole(networkId)
  return role === "owner" || role === "admin"
}
