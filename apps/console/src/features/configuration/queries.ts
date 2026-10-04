import {
  compareConfiguration,
  getConfiguration,
  getConfigurationRevision,
  getConfigurationSchema,
  getConfigurationSource,
  listConfigurationRevisions,
  type UserConfigurationView,
} from "@meshway/control-client"
import {
  infiniteQueryOptions,
  queryOptions,
  skipToken,
} from "@tanstack/react-query"

import { pollUnlessLive } from "@/features/live/status"
import { networkKey } from "@/features/network/queries"
import { ApiFailure, authed, client, problemCode } from "@/lib/api"

import { readSchema } from "./schema-check"

// Configuration reads. Only non-secret views enter the query cache; the
// JSONC source (secrets) is read by `readSource` into the draft alone.

const configurationKey = (networkId: string) =>
  [...networkKey(networkId), "configuration"] as const

/** Every configuration read of a network, for refreshing after a change. */
export const configurationReads = configurationKey

/** The current configuration, non-secret, with its revision. */
export const configurationQuery = (networkId: string) => {
  const poll = pollUnlessLive(networkId, 30_000)
  return queryOptions({
    queryKey: [...configurationKey(networkId), "current"],
    queryFn: ({ signal }) =>
      authed((token) =>
        getConfiguration(client, { networkId }, { token, signal })
      ),
    staleTime: 10_000,
    // A role the controller refuses stays refused until the role changes.
    // The event stream says when a new revision makes it stale.
    refetchInterval: (query) => (isDenied(query.state.error) ? false : poll()),
  })
}

/** The configuration a member sees: the settings of their own nodes. */
export type MemberConfiguration = Extract<
  UserConfigurationView,
  { type: "member" }
>

/**
 * A member's own nodes' settings. The controller has no read for them: they
 * come only in the event stream's configuration snapshot, which writes them
 * here. Owners and admins read the whole view (`configurationQuery`).
 */
export const memberConfigurationQuery = (networkId: string) =>
  queryOptions<MemberConfiguration>({
    queryKey: [...configurationKey(networkId), "member"],
    queryFn: skipToken,
    staleTime: Infinity,
  })

/** One revision's configuration, non-secret. Revisions never change. */
export const revisionQuery = (networkId: string, revision: number) =>
  queryOptions({
    queryKey: [...configurationKey(networkId), "revision", revision],
    queryFn: ({ signal }) =>
      authed((token) =>
        getConfigurationRevision(
          client,
          { networkId, revision },
          { token, signal }
        )
      ),
    staleTime: Infinity,
  })

/** Revisions per page; the controller takes 1 to 200. */
const REVISION_PAGE = 50

/** The revisions, newest first, a page at a time. */
export const revisionsQuery = (networkId: string) =>
  infiniteQueryOptions({
    queryKey: [...configurationKey(networkId), "revisions"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listConfigurationRevisions(
          client,
          { networkId },
          pageParam === null
            ? { limit: REVISION_PAGE }
            : { limit: REVISION_PAGE, cursor: pageParam },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 10_000,
  })

/** What changed from one revision to another, as the controller computes it. */
export const comparisonQuery = (
  networkId: string,
  fromRevision: number,
  toRevision: number
) =>
  queryOptions({
    queryKey: [
      ...configurationKey(networkId),
      "compare",
      fromRevision,
      toRevision,
    ],
    queryFn: ({ signal }) =>
      authed((token) =>
        compareConfiguration(
          client,
          { networkId },
          { fromRevision, toRevision },
          { token, signal }
        )
      ),
    staleTime: Infinity,
  })

/** The controller's configuration JSON Schema, for the editor's hints. */
export const schemaQuery = () =>
  queryOptions({
    queryKey: ["configuration", "schema"],
    queryFn: async ({ signal }) => {
      const response = await authed((token) =>
        getConfigurationSchema(client, { token, signal })
      )
      return readSchema(response.schemaJson)
    },
    staleTime: Infinity,
  })

/** The current JSONC source: secret, so it goes to the draft and never to the cache. */
export async function readSource(networkId: string, signal: AbortSignal) {
  const response = await authed((token) =>
    getConfigurationSource(client, { networkId }, {}, { token, signal })
  )
  return { revision: response.revision, text: response.sourceJsonc }
}

/** The controller refused the read for this account's role. */
export const isDenied = (error: unknown) =>
  error instanceof ApiFailure &&
  problemCode(error.failure) === "permission_denied"
