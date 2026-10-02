import type { AccountInvitation, AccountRole } from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

// What a role may do to memberships, as the controller decides it
// (`may_grant`): an Owner grants and changes any role; an Admin only invites
// and removes Members; a Member manages nothing. The last Owner can be
// neither demoted nor removed (409). These only choose which controls are
// offered; the controller still decides every request.

export const ROLES = [
  "owner",
  "admin",
  "member",
] as const satisfies readonly AccountRole[]

export const roleLabel: Record<AccountRole, () => string> = {
  owner: () => m.members_role_owner(),
  admin: () => m.members_role_admin(),
  member: () => m.members_role_member(),
}

const rank: Record<AccountRole, number> = { owner: 0, admin: 1, member: 2 }
export const byRole = (a: AccountRole, b: AccountRole) => rank[a] - rank[b]

/** The roles `viewer` may hand out in an invitation. */
export function grantableRoles(viewer: AccountRole | null): AccountRole[] {
  if (viewer === "owner") return [...ROLES]
  if (viewer === "admin") return ["member"]
  return []
}

/** Changing a role takes an Owner: an Admin may grant Member only, which a Member already is. */
export const canChangeRole = (viewer: AccountRole | null) => viewer === "owner"

/** Removing someone else from the network. */
export const canRemove = (viewer: AccountRole | null, target: AccountRole) =>
  viewer === "owner" || (viewer === "admin" && target === "member")

export type InvitationState = "pending" | "accepted" | "revoked" | "expired"

/** Where an invitation stands at `now` (Unix seconds). */
export function invitationState(
  invitation: AccountInvitation,
  now: number
): InvitationState {
  if (invitation.consumed_at !== null) return "accepted"
  if (invitation.revoked_at !== null) return "revoked"
  if (invitation.expires_at <= now) return "expired"
  return "pending"
}

/** When the invitation reached its state (or, while pending, when it expires). */
export function invitationTime(
  invitation: AccountInvitation,
  state: InvitationState
): number {
  if (state === "accepted") return invitation.consumed_at ?? 0
  if (state === "revoked") return invitation.revoked_at ?? 0
  return invitation.expires_at
}

const INVITATION_TOKEN = /^mwi_[0-9a-f]{64}$/

/**
 * The link that is handed over: the secret rides in the fragment, which the
 * browser never sends to a server. Null for a token not of that form.
 */
export function invitationLink(token: string): string | null {
  if (!INVITATION_TOKEN.test(token)) return null
  return `${window.location.origin}/invite#${new URLSearchParams({ token }).toString()}`
}

/** Unix seconds as a calendar date and time in the current locale. */
export const unixIso = (seconds: number) =>
  new Date(seconds * 1000).toISOString()
