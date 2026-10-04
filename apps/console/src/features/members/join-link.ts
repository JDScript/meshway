import type { JoinLinkView, MembershipRole } from "@meshway/control-client"

// A join link's state, from the times the controller reports. A link is
// used or revoked for good; until then it expires at `expiresAt`.

export type JoinLinkState = "pending" | "used" | "revoked" | "expired"

type Times = Pick<JoinLinkView, "consumedAt" | "revokedAt" | "expiresAt">

/** The state at `now` (milliseconds), and the time that state is about. */
export function joinLinkState(
  link: Times,
  now: number
): { state: JoinLinkState; at: string } {
  if (link.consumedAt !== null) return { state: "used", at: link.consumedAt }
  if (link.revokedAt !== null) return { state: "revoked", at: link.revokedAt }
  if (Date.parse(link.expiresAt) <= now)
    return { state: "expired", at: link.expiresAt }
  return { state: "pending", at: link.expiresAt }
}

/**
 * Whether revoking is offered: the controller's `actions.revoke`, as of its
 * read, while the link is still pending here (a link that expired since the
 * read has nothing left to revoke).
 */
export const revocable = (
  link: Pick<JoinLinkView, "actions">,
  state: JoinLinkState
) => link.actions.revoke && state === "pending"

/**
 * The role a new link starts with, among those the controller lets this
 * account issue (`NetworkActions.joinLinkRoles`): member when offered, else
 * the lowest offered; none when nothing may be issued.
 */
export function initialRole(
  roles: readonly MembershipRole[]
): MembershipRole | null {
  if (roles.includes("member")) return "member"
  if (roles.includes("admin")) return "admin"
  return roles.includes("owner") ? "owner" : null
}

/** How long a new link may be used for, in seconds; the controller takes 60 to 604800. */
export const JOIN_LINK_EXPIRIES = [3_600, 86_400, 259_200, 604_800] as const
export type JoinLinkExpiry = (typeof JOIN_LINK_EXPIRIES)[number]
