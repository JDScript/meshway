import type {
  EnrollmentKeyView,
  NodeInventoryView,
} from "@meshway/control-client"

// Pure facts about nodes and enrolment keys, from what the controller
// reports: nothing here is runtime state (this controller reports none).

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

type KeyFacts = Pick<
  EnrollmentKeyView,
  "consumedAt" | "consumedRequestId" | "revokedAt" | "expiresAt"
>

/**
 * A key's state at `now` (milliseconds), and the time it is about. A used
 * key stays used whatever came after (the controller refuses revoking it);
 * the controller records the use with its request and time together, so
 * either one says the key was used. Until then a revoked key is revoked,
 * and one past `expiresAt` expired.
 */
export function enrollmentKeyState(
  key: KeyFacts,
  now: number
): { state: EnrollmentKeyState; at: string | null } {
  if (key.consumedAt !== null || key.consumedRequestId != null)
    return { state: "consumed", at: key.consumedAt }
  if (key.revokedAt !== null) return { state: "revoked", at: key.revokedAt }
  if (Date.parse(key.expiresAt) <= now)
    return { state: "expired", at: key.expiresAt }
  return { state: "pending", at: key.expiresAt }
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
