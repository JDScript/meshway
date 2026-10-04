import type {
  EnrollmentKeyView,
  NodeIdentity,
  NodeInventoryView,
} from "@meshway/control-client"

// Pure facts about nodes and enrolment keys, from what the controller
// reports: nothing here is runtime state (this controller reports none).

/** The tags or the user a node acts as, for searching and showing. */
export const identityWords = (identity: NodeIdentity): string[] =>
  identity.type === "tag" ? identity.tags : [identity.user]

/** Nodes by hostname, then ID; those matching `q` (a substring of the hostname). */
export function shownNodes(
  nodes: readonly NodeInventoryView[],
  q: string
): NodeInventoryView[] {
  const needle = q.trim().toLowerCase()
  return nodes
    .filter(
      (node) => needle === "" || node.hostname.toLowerCase().includes(needle)
    )
    .sort(
      (a, b) => a.hostname.localeCompare(b.hostname) || a.id.localeCompare(b.id)
    )
}

/** Whether either of the node's current keys is revoked. */
export const keyRevoked = (node: NodeInventoryView) =>
  node.controlKeyRevokedAt !== null || node.dataplaneKeyRevokedAt !== null

// Enrolment keys ---------------------------------------------------------

export type EnrollmentKeyState = "pending" | "consumed" | "revoked" | "expired"

type KeyTimes = Pick<
  EnrollmentKeyView,
  "consumedRequestId" | "revokedAt" | "expiresAt"
>

/**
 * A key's state at `now` (milliseconds), and the time it is about (none
 * for a used key: the controller does not report when). Used keys stay
 * used even if revoked later; until then a key expires at `expiresAt`.
 */
export function enrollmentKeyState(
  key: KeyTimes,
  now: number
): { state: EnrollmentKeyState; at: string | null } {
  if (key.consumedRequestId != null) return { state: "consumed", at: null }
  if (key.revokedAt !== null) return { state: "revoked", at: key.revokedAt }
  if (Date.parse(key.expiresAt) <= now)
    return { state: "expired", at: key.expiresAt }
  return { state: "pending", at: key.expiresAt }
}

/** Pending keys first, the soonest to expire on top; then the rest, newest first. */
export function byEnrollmentKeyState(
  a: {
    state: EnrollmentKeyState
    key: Pick<EnrollmentKeyView, "createdAt" | "expiresAt">
  },
  b: {
    state: EnrollmentKeyState
    key: Pick<EnrollmentKeyView, "createdAt" | "expiresAt">
  }
): number {
  const pendingA = a.state === "pending"
  const pendingB = b.state === "pending"
  if (pendingA !== pendingB) return pendingA ? -1 : 1
  if (pendingA) return Date.parse(a.key.expiresAt) - Date.parse(b.key.expiresAt)
  return Date.parse(b.key.createdAt) - Date.parse(a.key.createdAt)
}

/** How long a new key may be used for, in seconds; the controller takes up to 30 days. */
export const KEY_EXPIRIES = [3_600, 86_400, 604_800, 2_592_000] as const
export type KeyExpiry = (typeof KEY_EXPIRIES)[number]

const LABEL = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/

/**
 * A hostname as typed, in the form the controller keeps (one DNS label,
 * lowercase); "" for none (the device's own name is used); null when it
 * is not one.
 */
export function hostnameOf(raw: string): string | null {
  const value = raw.trim().toLowerCase()
  if (value === "") return ""
  return LABEL.test(value) ? value : null
}
