import { listAccountInvitations } from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { authed } from "@/lib/api"

/** Invitation metadata (never the secret); owners see all, admins member invitations only. */
export const invitationsQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "invitations"],
    queryFn: () =>
      authed((base, token) => listAccountInvitations(base, token, networkId)),
    staleTime: 30_000,
  })
