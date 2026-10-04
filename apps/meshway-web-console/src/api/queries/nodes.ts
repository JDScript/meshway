import {
  getNodeSettings,
  getTransportCatalog,
  listEnrollmentKeys,
  listNodes,
} from "@meshway/control-client"
import { infiniteQueryOptions, queryOptions } from "@tanstack/react-query"

import { authed, client } from "@/api/client"
import { pollUnlessLive } from "@/state/live"

import { isDenied } from "./configuration"
import { networkKey } from "./network"

/**
 * The network's nodes as the controller lists them to this account: every
 * node for owners and admins, a member's own User-identity devices for
 * members. Registration and keys only; no runtime state. The event stream
 * writes it (its entities' nodes); it is polled when the stream is not live.
 */
export const nodesQuery = (networkId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "nodes"],
    queryFn: ({ signal }) =>
      authed((token) => listNodes(client, { networkId }, { token, signal })),
    staleTime: 10_000,
    refetchInterval: pollUnlessLive(networkId, 30_000),
  })

/** Enrolment keys per page; the controller takes 1 to 200. */
const KEY_PAGE = 50

/**
 * The network's enrolment keys, newest first, a page at a time, with the
 * controller's total; metadata only (a key's secret is never readable
 * again). Owners and admins.
 */
export const enrollmentKeysQuery = (networkId: string) =>
  infiniteQueryOptions({
    queryKey: [...networkKey(networkId), "enrollment-keys"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listEnrollmentKeys(
          client,
          { networkId },
          pageParam === null
            ? { limit: KEY_PAGE }
            : { limit: KEY_PAGE, cursor: pageParam },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 10_000,
    refetchInterval: (query) => (isDenied(query.state.error) ? false : 30_000),
  })

/**
 * One node's settings as the controller reads them to this account, with
 * the revision they belong to: owners and admins any node, a member their
 * own User-identity devices (others are not found). The event stream does
 * not write it; it is read again when the revision changes.
 */
export const nodeSettingsQuery = (networkId: string, nodeId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "nodes", nodeId, "settings"],
    queryFn: ({ signal }) =>
      authed((token) =>
        getNodeSettings(client, { networkId, nodeId }, { token, signal })
      ),
    staleTime: 10_000,
    refetchInterval: pollUnlessLive(networkId, 30_000),
  })

/** The transports this controller manages. */
export const transportCatalogQuery = () =>
  queryOptions({
    queryKey: ["transport-catalog"],
    queryFn: ({ signal }) =>
      authed((token) => getTransportCatalog(client, { token, signal })),
    staleTime: 5 * 60_000,
  })
