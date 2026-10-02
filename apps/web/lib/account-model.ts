import type { AccountRole } from "@meshway/control-client"

// Serializable account views and action results. None of them ever carries a
// password, a session, an invitation secret or the administrator token.

/** Who the console is signed in as; derived on the server from the controller. */
export type ConsoleIdentity =
  | { kind: "bootstrap" }
  | { kind: "user"; email: string; memberships: { networkId: string; role: AccountRole }[] }
  | { kind: "user-unverified"; error: string }

/** `email` is echoed back so a failed attempt keeps it; the password never is. */
export type AccountLoginState =
  | { status: "idle" }
  | { status: "malformed"; message: string; email: string }
  | { status: "invalid"; email: string }
  | { status: "limited"; email: string }
  | { status: "unavailable"; detail: string; email: string }

export type OwnerClaimState =
  | { status: "idle" }
  | { status: "error"; message: string; email: string }
  | { status: "claimed"; email: string; networkId: string; note: string | null }

export type InvitationState =
  | { status: "idle" }
  | { status: "error"; message: string }

export type InvitationIssueState =
  | { status: "idle" }
  | { status: "error"; message: string }
  /** `path` holds the one-time secret in its fragment; shown once, never stored. */
  | { status: "created"; email: string; role: AccountRole; expiresAt: number; path: string }

export const roleLabels: Record<AccountRole, string> = {
  owner: "所有者",
  admin: "管理员",
  member: "成员",
}

/** The role this identity holds in `networkId`, or null when it has none (or is not a user). */
export function roleIn(identity: ConsoleIdentity, networkId: string): AccountRole | null {
  if (identity.kind !== "user") return null
  return identity.memberships.find((membership) => membership.networkId === networkId)?.role ?? null
}

/** Controller passwords are 12–1024 UTF-8 bytes. */
export function passwordProblem(password: string): string | null {
  const bytes = new TextEncoder().encode(password).length
  if (bytes < 12) return "密码至少 12 个字节。"
  if (bytes > 1024) return "密码最多 1024 个字节。"
  return null
}

const INVITATION_TOKEN = /^mwi_[0-9a-f]{64}$/

/** Invitation links carry the secret in the URL fragment, which is never sent to a server. */
export function invitationPath(token: string): string {
  if (!INVITATION_TOKEN.test(token)) throw new Error("Invalid invitation token")
  return `/invite#${new URLSearchParams({ token })}`
}

/** Reads the secret back from an invitation link fragment (client side). */
export function invitationTokenFromHash(hash: string): string | null {
  const token = new URLSearchParams(hash.replace(/^#/, "")).get("token")
  return token !== null && INVITATION_TOKEN.test(token) ? token : null
}
