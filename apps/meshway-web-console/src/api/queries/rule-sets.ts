import {
  getRuleSetReport,
  listRuleSetCatalog,
  listRuleSetEntries,
  listRuleSets,
  matchRuleSet,
  refreshRuleSet,
  type RuleSetEntryKind,
} from "@meshway/control-client"
import {
  infiniteQueryOptions,
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

/**
 * The entries a set's bound content skipped, by the digest it is for: the
 * same content always has the same report, so it is read once.
 */
export const ruleSetReportQuery = (
  networkId: string,
  ruleSetKey: string,
  digest: string
) =>
  queryOptions({
    queryKey: [...ruleSetsKey(networkId), "report", ruleSetKey, digest],
    queryFn: ({ signal }) =>
      authed((token) =>
        getRuleSetReport(client, { networkId, ruleSetKey }, { token, signal })
      ),
    staleTime: Infinity,
  })

const pageOf = (cursor: string | null) => (cursor === null ? {} : { cursor })

/**
 * One provider's sets in the newest catalog the controller verified, by
 * name prefix (`q`), a page at a time; the picker reads one short page.
 * 409 `rule_set_catalog_not_ready` until the controller has verified it.
 */
export const ruleSetCatalogQuery = (
  networkId: string,
  provider: string,
  q: string,
  limit = 100
) =>
  infiniteQueryOptions({
    queryKey: [...ruleSetsKey(networkId), "catalog", provider, q, limit],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listRuleSetCatalog(
          client,
          { networkId },
          { provider, limit, ...(q === "" ? {} : { q }), ...pageOf(pageParam) },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 60_000,
  })

/** One kind of a set's bound members (`digest`), a page at a time; the same content always lists the same. */
export const ruleSetEntriesQuery = (
  networkId: string,
  ruleSetKey: string,
  digest: string,
  kind: RuleSetEntryKind,
  q: string
) =>
  infiniteQueryOptions({
    queryKey: [
      ...ruleSetsKey(networkId),
      "entries",
      ruleSetKey,
      digest,
      kind,
      q,
    ],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listRuleSetEntries(
          client,
          { networkId, ruleSetKey },
          { kind, ...(q === "" ? {} : { q }), ...pageOf(pageParam) },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: Infinity,
  })

/** Whether a set's bound content (`digest`) holds a name or an address, and through which member. */
export const ruleSetMatchQuery = (
  networkId: string,
  ruleSetKey: string,
  digest: string,
  query: { name: string } | { ip: string }
) =>
  queryOptions({
    queryKey: [...ruleSetsKey(networkId), "match", ruleSetKey, digest, query],
    queryFn: ({ signal }) =>
      authed((token) =>
        matchRuleSet(client, { networkId, ruleSetKey }, query, {
          token,
          signal,
        })
      ),
    staleTime: Infinity,
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
