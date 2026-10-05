import {
  getLeaveImpact,
  getMemberRemovalImpact,
  getMemberRoleChangeImpact,
  listJoinLinks,
  listNetworkMembers,
  type MembershipRole,
} from "@meshway/control-client"
import { infiniteQueryOptions, queryOptions } from "@tanstack/react-query"

import { authed, client } from "@/api/client"

import { networkKey } from "./network"

/** Rows per page; the controller takes 1 to 200. */
const MEMBER_PAGE = 100
const LINK_PAGE = 100

/**
 * The network's members, a page at a time, each with what this account may
 * do to it (`actions`). Only those who manage members may read them; the
 * controller answers others 403.
 */
export const membersQuery = (networkId: string) =>
  infiniteQueryOptions({
    queryKey: [...networkKey(networkId), "members"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listNetworkMembers(
          client,
          { networkId },
          pageParam === null
            ? { limit: MEMBER_PAGE }
            : { limit: MEMBER_PAGE, cursor: pageParam },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 30_000,
  })

/** The network's join links, metadata only (a link's secret is never readable again). */
export const joinLinksQuery = (networkId: string) =>
  infiniteQueryOptions({
    queryKey: [...networkKey(networkId), "join-links"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listJoinLinks(
          client,
          { networkId },
          pageParam === null
            ? { limit: LINK_PAGE }
            : { limit: LINK_PAGE, cursor: pageParam },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 30_000,
  })

/**
 * What removing a member does now: the devices it deletes, and the
 * membership version and configuration revision the removal must be sent
 * against. Read when its confirmation opens, never reused from before.
 */
export const removalImpactQuery = (networkId: string, membershipId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "removal-impact", membershipId],
    queryFn: ({ signal }) =>
      authed((token) =>
        getMemberRemovalImpact(
          client,
          { networkId, membershipId },
          { token, signal }
        )
      ),
    staleTime: 0,
    gcTime: 0,
  })

/** The same for leaving: this account's own membership. */
export const leaveImpactQuery = (networkId: string) =>
  queryOptions({
    queryKey: [...networkKey(networkId), "leave-impact"],
    queryFn: ({ signal }) =>
      authed((token) =>
        getLeaveImpact(client, { networkId }, { token, signal })
      ),
    staleTime: 0,
    gcTime: 0,
  })

/**
 * What changing a member to `role` does now: the join links it revokes
 * (theirs, unused and unexpired, that the new role may not issue), and the
 * membership version the change must be sent against. Read when its
 * confirmation opens, never reused from before.
 */
export const roleChangeImpactQuery = (
  networkId: string,
  membershipId: string,
  role: MembershipRole
) =>
  queryOptions({
    queryKey: [
      ...networkKey(networkId),
      "role-change-impact",
      membershipId,
      role,
    ],
    queryFn: ({ signal }) =>
      authed((token) =>
        getMemberRoleChangeImpact(
          client,
          { networkId, membershipId, role },
          { token, signal }
        )
      ),
    staleTime: 0,
    gcTime: 0,
  })
