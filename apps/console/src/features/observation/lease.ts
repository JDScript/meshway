import type { ObservationTask } from "@meshway/control-client"

import { isActive } from "./record.ts"

// When to renew a subscription: at the controller's `renewAfter` (a renewal
// before it is refused), and only before its lease (`subscriberExpiresAt`)
// runs out. The page renews only while it is visible; a lease that ran out
// is ended and is not renewed. Times are the controller's, read against
// this browser's clock.

/** Past `renewAfter`, so a clock a little ahead of the controller's is not refused. */
export const RENEW_MARGIN_MS = 250
/** After a renewal refused as early: try again this much later, while the lease lasts. */
export const RENEW_RETRY_MS = 1_000

export type LeaseTimes = Pick<
  ObservationTask,
  "phase" | "renewAfter" | "subscriberExpiresAt" | "expiresAt"
>

export type LeasePlan =
  /** Renew in `inMs`. */
  | { type: "renew"; inMs: number }
  /** The lease ran out: the subscription has ended. */
  | { type: "lapsed" }
  /** Nothing to renew: the task has ended. */
  | { type: "none" }

export const leaseExpiry = (task: LeaseTimes) =>
  Date.parse(task.subscriberExpiresAt ?? task.expiresAt)

export function leasePlan(task: LeaseTimes, now: number): LeasePlan {
  if (!isActive(task.phase)) return { type: "none" }
  const expires = leaseExpiry(task)
  if (!Number.isFinite(expires) || now >= expires) return { type: "lapsed" }
  const after = Date.parse(task.renewAfter)
  const due = Number.isFinite(after) ? after + RENEW_MARGIN_MS : now
  const at = Math.min(due, expires - RENEW_MARGIN_MS)
  return { type: "renew", inMs: Math.max(0, at - now) }
}

/** After a renewal refused as early, the next try; lapsed when none fits before the lease ends. */
export function retryPlan(task: LeaseTimes, now: number): LeasePlan {
  if (!isActive(task.phase)) return { type: "none" }
  const expires = leaseExpiry(task)
  if (!Number.isFinite(expires)) return { type: "lapsed" }
  const inMs = Math.min(RENEW_RETRY_MS, expires - RENEW_MARGIN_MS - now)
  return inMs > 0 ? { type: "renew", inMs } : { type: "lapsed" }
}
