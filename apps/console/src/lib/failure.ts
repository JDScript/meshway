import { ControlApiError } from "@meshway/control-client"
import { toast } from "sonner"

import { SessionRejected } from "@/lib/api"
import { m } from "@/paraglide/messages.js"

/**
 * What a failed call means to the person who made it. The controller's
 * error bodies are not read (the client drops them), so this is the status
 * alone; a lost response is not a refusal: the change may have happened.
 */
type Failure =
  | "unauthorized"
  | "forbidden"
  | "conflict"
  | "invalid"
  | "missing"
  | "limited"
  | "unknown"

/** null when there is nothing to say here: a rejected session hands over to the sign-in. */
export function failureOf(error: unknown): Failure | null {
  if (error instanceof SessionRejected) return null
  if (!(error instanceof ControlApiError)) return "unknown"
  // Only calls made without the tab's session (a sign-in, an invitation, a reset) see this: `authed` turns it into SessionRejected.
  if (error.status === 401) return "unauthorized"
  if (error.status === 403) return "forbidden"
  if (error.status === 409) return "conflict"
  if (error.status === 400 || error.status === 413 || error.status === 422)
    return "invalid"
  if (error.status === 404) return "missing"
  if (error.status === 429) return "limited"
  return "unknown"
}

const failureMessage: Record<Failure, () => string> = {
  unauthorized: () => m.failure_unauthorized(),
  forbidden: () => m.failure_forbidden(),
  conflict: () => m.failure_conflict(),
  invalid: () => m.failure_invalid(),
  missing: () => m.failure_missing(),
  limited: () => m.failure_limited(),
  unknown: () => m.failure_unknown(),
}

/** What a call's own context says about some failures (a 409 on the last Owner). */
type FailureText = Partial<Record<Failure, () => string>>

/** The words for a failure, the caller's own where it has them; null when there is nothing to say. */
export function failureText(
  error: unknown,
  overrides: FailureText = {}
): string | null {
  const failure = failureOf(error)
  if (!failure) return null
  return (overrides[failure] ?? failureMessage[failure])()
}

/** Announces a failed change as a toast. */
export function toastFailure(error: unknown) {
  const text = failureText(error)
  if (text) toast.error(text)
}

/** `toastFailure` with the caller's own words for some failures. */
export const toastFailureWith =
  (overrides: FailureText) => (error: unknown) => {
    const text = failureText(error, overrides)
    if (text) toast.error(text)
  }
