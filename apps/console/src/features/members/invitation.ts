import type { AccountInvitation } from "@meshway/control-client"

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

/** When the invitation reached its state (or, while pending, when it expires), in Unix seconds. */
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
