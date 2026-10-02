import {
  listAccountInvitations,
  listAccountMembers,
  listNodeOwnership,
} from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { authed } from "@/lib/api"

/** Live memberships; every member may read them. */
export const membersQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "members"],
    queryFn: () =>
      authed((base, token) => listAccountMembers(base, token, networkId)),
    staleTime: 30_000,
  })

/** Invitation metadata (never the secret); owners see all, admins member invitations only. */
export const invitationsQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "invitations"],
    queryFn: () =>
      authed((base, token) => listAccountInvitations(base, token, networkId)),
    staleTime: 30_000,
  })

/** Which membership owns each node; null is a node no membership owns yet. */
export const ownershipQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "ownership"],
    queryFn: () =>
      authed((base, token) => listNodeOwnership(base, token, networkId)),
    staleTime: 30_000,
  })
