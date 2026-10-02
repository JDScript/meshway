import type {
  AccountInvitation,
  AccountMember,
  AccountMembershipChange,
  AccountRole,
  NodeOwnership,
  NodeSummary,
} from "@meshway/control-client"

// Pure views over the controller's member, invitation and ownership reads.
// Permissions here only decide which controls are offered; the controller
// enforces the role ceiling inside the write transaction either way.

/** Who is acting on this network's members. */
export type MemberActor =
  | { kind: "bootstrap" }
  | { kind: "user"; userId: string; role: AccountRole | null }

const roleRank: Record<AccountRole, number> = { owner: 0, admin: 1, member: 2 }

export function canManageMembers(actor: MemberActor): boolean {
  return actor.kind === "bootstrap" || actor.role === "owner" || actor.role === "admin"
}

/** Roles an invitation from this actor may carry: Owner every role, Admin only Member. */
export function grantableRoles(actor: MemberActor): AccountRole[] {
  if (actor.kind === "bootstrap" || actor.role === "owner") return ["member", "admin", "owner"]
  if (actor.role === "admin") return ["member"]
  return []
}

/**
 * Changes this actor may offer for one member: an Owner (or the instance
 * credential) any other role or removal; an Admin only removes Members.
 * Removing yourself is "leave", a separate action.
 */
export function memberChanges(actor: MemberActor, target: AccountMember): AccountMembershipChange[] {
  const current = target.membership.role
  if (actor.kind === "user" && actor.userId === target.user.user_id) {
    return actor.role === "owner" ? (["owner", "admin", "member"] as const).filter((role) => role !== current) : []
  }
  if (actor.kind === "bootstrap" || actor.role === "owner") {
    return [...(["owner", "admin", "member"] as const).filter((role) => role !== current), "remove"]
  }
  if (actor.role === "admin" && current === "member") return ["remove"]
  return []
}

/** Owners first, then Admins, then Members; by email within a role. */
export function sortMembers(members: AccountMember[]): AccountMember[] {
  return [...members].sort(
    (a, b) => roleRank[a.membership.role] - roleRank[b.membership.role] || a.user.email.localeCompare(b.user.email)
  )
}

export type InvitationStatus = "pending" | "accepted" | "revoked" | "expired"

/** `now` in Unix seconds, like the controller's timestamps. */
export function invitationStatus(invitation: AccountInvitation, now: number): InvitationStatus {
  if (invitation.revoked_at !== null) return "revoked"
  if (invitation.consumed_at !== null) return "accepted"
  if (invitation.expires_at <= now) return "expired"
  return "pending"
}

export const invitationStatusLabels: Record<InvitationStatus, string> = {
  pending: "待接受",
  accepted: "已接受",
  revoked: "已撤销",
  expired: "已过期",
}

/** Pending first, newest expiry first within each status. */
export function sortInvitations(invitations: AccountInvitation[], now: number): AccountInvitation[] {
  const rank: Record<InvitationStatus, number> = { pending: 0, accepted: 1, expired: 2, revoked: 3 }
  return [...invitations].sort(
    (a, b) => rank[invitationStatus(a, now)] - rank[invitationStatus(b, now)] || b.expires_at - a.expires_at
  )
}

export type OwnershipView = {
  /** Membership ID → that member's devices (active and revoked), inventory order. */
  byMembership: Map<string, NodeSummary[]>
  /** Legacy nodes nobody owns yet that are still active: an Owner may assign them. */
  unassigned: NodeSummary[]
  /** Owned by a membership that is no longer live (removed or left). */
  formerMember: NodeSummary[]
}

export function ownershipView(ownership: NodeOwnership[], members: AccountMember[], nodes: NodeSummary[]): OwnershipView {
  const live = new Set(members.map((member) => member.membership.membership_id))
  const owner = new Map(ownership.map((entry) => [entry.node_id, entry.membership_id]))
  const view: OwnershipView = { byMembership: new Map(), unassigned: [], formerMember: [] }
  for (const node of nodes) {
    if (!owner.has(node.id)) continue
    const membershipId = owner.get(node.id) ?? null
    if (membershipId === null) {
      if (node.revoked_at === null) view.unassigned.push(node)
      continue
    }
    if (!live.has(membershipId)) {
      view.formerMember.push(node)
      continue
    }
    const list = view.byMembership.get(membershipId) ?? []
    list.push(node)
    view.byMembership.set(membershipId, list)
  }
  return view
}

export type NodeOwner =
  | { kind: "member"; email: string; role: AccountRole }
  | { kind: "unassigned" }
  | { kind: "former" }
  | { kind: "unknown" }

/** Node ID → who owns it, for inventory columns. */
export function nodeOwners(ownership: NodeOwnership[], members: AccountMember[]): Map<string, NodeOwner> {
  const byMembership = new Map(members.map((member) => [member.membership.membership_id, member]))
  const owners = new Map<string, NodeOwner>()
  for (const entry of ownership) {
    if (entry.membership_id === null) {
      owners.set(entry.node_id, { kind: "unassigned" })
      continue
    }
    const member = byMembership.get(entry.membership_id)
    owners.set(entry.node_id, member ? { kind: "member", email: member.user.email, role: member.membership.role } : { kind: "former" })
  }
  return owners
}

const RESET_TOKEN = /^mwr_[0-9a-f]{64}$/

/** Password reset links carry the secret in the URL fragment, never sent to a server. */
export function resetPath(token: string): string {
  if (!RESET_TOKEN.test(token)) throw new Error("Invalid reset token")
  return `/reset-password#${new URLSearchParams({ token })}`
}

export function resetTokenFromHash(hash: string): string | null {
  const token = new URLSearchParams(hash.replace(/^#/, "")).get("token")
  return token !== null && RESET_TOKEN.test(token) ? token : null
}
