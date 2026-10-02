import { getNetworkPolicy } from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { authed } from "@/lib/api"

// The policy is an editing source: read on navigation and after a save,
// never polled (a draft keeps the revision it was loaded at anyway).

export const policyQuery = (networkId: string) =>
  queryOptions({
    queryKey: ["network", networkId, "policy"],
    queryFn: () =>
      authed((base, token) => getNetworkPolicy(base, token, networkId)),
    staleTime: 30_000,
  })
