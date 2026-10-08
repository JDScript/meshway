import { listRuleSets, refreshRuleSet } from "@meshway/control-client"
import {
  queryOptions,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query"

import { authed, client } from "@/api/client"
import { attempts } from "@/state/attempts"

import { networkKey } from "./network"

// The rule sets' status as the controller holds it for the current
// revision (Owner / Admin only). The event stream invalidates this key on
// `resources.changed`; a refresh writes its own result back.

export const ruleSetsKey = (networkId: string) =>
  [...networkKey(networkId), "rule-sets"] as const

export const ruleSetsQuery = (networkId: string) =>
  queryOptions({
    queryKey: ruleSetsKey(networkId),
    queryFn: ({ signal }) =>
      authed((token) => listRuleSets(client, { networkId }, { token, signal })),
    staleTime: 10_000,
  })

/** By network and set: a refresh whose outcome is unknown is sent again with its ID. */
const refreshes = attempts<Record<string, never>>("rule-set-refresh")

/**
 * Downloads and converts one set again; its view is the controller's
 * answer. Trying again after a refresh whose outcome is unknown sends the
 * same request ID, which the controller answers with the set's current
 * view instead of downloading again.
 */
export function useRefreshRuleSet(networkId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (ruleSetKey: string) =>
      refreshes.send(
        `${networkId}/${ruleSetKey}`,
        ruleSetKey,
        () => ({}),
        (attempt) =>
          authed((token) =>
            refreshRuleSet(
              client,
              { networkId, ruleSetKey },
              { requestId: attempt.requestId },
              { token }
            )
          )
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: ruleSetsKey(networkId) }),
  })
}
