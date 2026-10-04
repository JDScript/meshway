// A device's enrolment request as the browser approval page shows it. The
// controller's phase is the request's state; the page only notices the
// expiry passing while it is open (the controller then reads it as expired).
// A pending request has no user: this page approves it or leaves it (the
// device cancels it, or it expires). Only the account that approved it may
// withdraw the approval, until the device redeems it.

export type DevicePhase =
  "pending" | "approved" | "withdrawn" | "cancelled" | "consumed" | "expired"

/** The phase to show at `now`: a pending or approved request past its expiry is expired. */
export function phaseAt(
  request: { phase: DevicePhase; expiresAt: string },
  now: number
): DevicePhase {
  const open = request.phase === "pending" || request.phase === "approved"
  return open && Date.parse(request.expiresAt) <= now
    ? "expired"
    : request.phase
}

/** An approved request waits for the device to redeem it: read it again until it settles. */
export const awaitingDevice = (phase: DevicePhase) => phase === "approved"

type Approval = { userId: string; networkId: string } | null

/** Who approved the request, as this account may know: itself, another account, or not said. */
export type Approver = "self" | "other" | "unknown"

export const approverOf = (approval: Approval, accountId: string): Approver =>
  approval === null
    ? "unknown"
    : approval.userId === accountId
      ? "self"
      : "other"

/** What the page offers for the request at `phase`. */
export type DeviceOffer =
  /** Approve into one of `networks` (empty: none of the account's networks may take it). */
  | { kind: "approve"; networks: string[] }
  /** Withdraw this account's own approval. */
  | { kind: "withdraw" }
  | { kind: "none" }

/**
 * A pending request is approved into one of the account's networks, or for
 * a re-login (`existing`) only into the node's own network; an approved one
 * may be withdrawn only by the account that approved it. Nothing else.
 */
export function offerFor(
  view: {
    request: { existing: { networkId: string } | null }
    approval: Approval
  },
  phase: DevicePhase,
  accountId: string,
  networkIds: readonly string[]
): DeviceOffer {
  if (phase === "pending") {
    const existing = view.request.existing
    return {
      kind: "approve",
      networks: existing
        ? networkIds.filter((id) => id === existing.networkId)
        : [...networkIds],
    }
  }
  if (phase === "approved" && approverOf(view.approval, accountId) === "self")
    return { kind: "withdraw" }
  return { kind: "none" }
}

/** Milliseconds until the request expires (never below 0). */
export const untilExpiry = (expiresAt: string, now: number) =>
  Math.max(0, Date.parse(expiresAt) - now)

/** A public key's hex in groups of four, to compare with what the device shows. */
export function keyGroups(hex: string): string {
  return (hex.toLowerCase().match(/.{1,4}/g) ?? []).join(" ")
}
