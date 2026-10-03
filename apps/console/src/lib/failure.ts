import type { Problem } from "@meshway/control-client"
import { toast } from "sonner"

import { ApiFailure, SessionRejected, type Failure } from "@/lib/api"
import { m } from "@/paraglide/messages.js"

// What a failed call means to the person who made it. The controller's
// problem code says it when the client could decode the body; otherwise the
// kind of failure and the HTTP status do. A lost response is not a refusal:
// the change may have happened.

type ProblemCode = Problem["code"]

const problemMessage: Record<ProblemCode, () => string> = {
  invalid_credentials: () => m.problem_invalid_credentials(),
  invalid_session: () => m.problem_invalid_session(),
  registration_disabled: () => m.problem_registration_disabled(),
  email_taken: () => m.problem_email_taken(),
  password_weak: () => m.problem_password_weak(),
  invalid_request: () => m.problem_invalid_request(),
  request_too_large: () => m.problem_request_too_large(),
  unsupported_media_type: () => m.problem_unsupported(),
  busy: () => m.problem_busy(),
  service_unavailable: () => m.problem_service_unavailable(),
  not_found: () => m.problem_not_found(),
  method_not_allowed: () => m.problem_unsupported(),
  permission_denied: () => m.problem_permission_denied(),
  request_id_conflict: () => m.problem_request_id_conflict(),
  name_conflict: () => m.problem_name_conflict(),
  invalid_cursor: () => m.problem_invalid_cursor(),
  version_conflict: () => m.problem_version_conflict(),
  last_owner: () => m.problem_last_owner(),
  already_member: () => m.problem_already_member(),
  invalid_join_link: () => m.problem_invalid_join_link(),
  revision_conflict: () => m.problem_revision_conflict(),
  configuration_invalid: () => m.problem_configuration_invalid(),
  state_conflict: () => m.problem_state_conflict(),
  state_limit: () => m.problem_state_limit(),
  observation_limit: () => m.problem_observation_limit(),
}

/** A call's own words for some problem codes (`invalid_request` on a sign-in form). */
export type FailureText = Partial<Record<ProblemCode, () => string>>

function failureMessage(failure: Failure, overrides: FailureText): string {
  switch (failure.kind) {
    case "http": {
      const code = failure.problem?.problem.code
      if (code) return (overrides[code] ?? problemMessage[code])()
      // A gateway in front of the controller answers for it when it is unreachable.
      if (failure.status === 502 || failure.status === 504)
        return m.failure_network()
      return m.failure_status({ status: String(failure.status) })
    }
    case "network":
      return m.failure_network()
    case "decode":
      return m.failure_decode()
    case "body_limit":
      return m.failure_body_limit()
    case "configuration":
    case "request":
      return m.failure_request()
    case "cancelled":
      return ""
  }
}

/** The words for a failure, the caller's own where it has them; null when there is nothing to say. */
export function failureText(
  error: unknown,
  overrides: FailureText = {}
): string | null {
  // A rejected session hands over to the sign-in, which says so itself.
  if (error instanceof SessionRejected) return null
  if (!(error instanceof ApiFailure)) return m.failure_unknown()
  return failureMessage(error.failure, overrides) || null
}

/** Announces a failed change as a toast. */
export function toastFailure(error: unknown) {
  const text = failureText(error)
  if (text) toast.error(text)
}

/** `toastFailure` with the caller's own words for some problem codes. */
export const toastFailureWith =
  (overrides: FailureText) => (error: unknown) => {
    const text = failureText(error, overrides)
    if (text) toast.error(text)
  }
