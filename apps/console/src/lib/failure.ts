import { ControlApiError } from "@meshway/control-client"
import { toast } from "sonner"

import { SessionRejected } from "@/lib/api"
import { m } from "@/paraglide/messages.js"

/**
 * What a failed call means to the person who made it. The controller's
 * error bodies are not read (the client drops them), so this is the status
 * alone; a lost response is not a refusal: the change may have happened.
 */
export type Failure =
  "forbidden" | "conflict" | "invalid" | "missing" | "unknown"

/** null when there is nothing to say here: a rejected session hands over to the sign-in. */
export function failureOf(error: unknown): Failure | null {
  if (error instanceof SessionRejected) return null
  if (!(error instanceof ControlApiError)) return "unknown"
  if (error.status === 403) return "forbidden"
  if (error.status === 409) return "conflict"
  if (error.status === 400 || error.status === 413 || error.status === 422)
    return "invalid"
  if (error.status === 404) return "missing"
  return "unknown"
}

export const failureMessage: Record<Failure, () => string> = {
  forbidden: () => m.failure_forbidden(),
  conflict: () => m.failure_conflict(),
  invalid: () => m.failure_invalid(),
  missing: () => m.failure_missing(),
  unknown: () => m.failure_unknown(),
}

/** Announces a failed change as a toast. */
export function toastFailure(error: unknown) {
  const failure = failureOf(error)
  if (failure) toast.error(failureMessage[failure]())
}
