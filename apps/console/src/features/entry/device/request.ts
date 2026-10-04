// A device's enrolment request as the browser approval page shows it. The
// controller's phase is the request's state; the page only notices the
// expiry passing while it is open (the controller then reads it as expired).

export type DevicePhase =
  "pending" | "approved" | "denied" | "consumed" | "expired"

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

/** Only a pending request can be approved or denied from this page. */
export const decidable = (phase: DevicePhase) => phase === "pending"

/** An approved request waits for the device to redeem it: read it again until it settles. */
export const awaitingDevice = (phase: DevicePhase) => phase === "approved"

/** Milliseconds until the request expires (never below 0). */
export const untilExpiry = (expiresAt: string, now: number) =>
  Math.max(0, Date.parse(expiresAt) - now)

/** A public key's hex in groups of four, to compare with what the device shows. */
export function keyGroups(hex: string): string {
  return (hex.toLowerCase().match(/.{1,4}/g) ?? []).join(" ")
}
