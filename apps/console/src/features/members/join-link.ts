import type { JoinLinkView } from "@meshway/control-client"

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

/** Pending links first, the soonest to expire on top; then the rest, latest first. */
export function byJoinLinkState(
  a: { state: JoinLinkState; at: string },
  b: { state: JoinLinkState; at: string }
): number {
  const pendingA = a.state === "pending"
  const pendingB = b.state === "pending"
  if (pendingA !== pendingB) return pendingA ? -1 : 1
  const order = Date.parse(a.at) - Date.parse(b.at)
  return pendingA ? order : -order
}

/** How long a new link may be used for, in seconds; the controller takes 60 to 604800. */
export const JOIN_LINK_EXPIRIES = [3_600, 86_400, 259_200, 604_800] as const
export type JoinLinkExpiry = (typeof JOIN_LINK_EXPIRIES)[number]
