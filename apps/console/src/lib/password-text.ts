import { m } from "@/paraglide/messages.js"

import {
  PASSWORD_MAX_BYTES,
  PASSWORD_MIN_BYTES,
  type PasswordProblem,
} from "./password"

// Kept apart from the rules (`password.ts`) so those stay testable without the messages.

export const passwordProblemText: Record<PasswordProblem, () => string> = {
  short: () => m.password_short({ min: String(PASSWORD_MIN_BYTES) }),
  long: () => m.password_long({ max: String(PASSWORD_MAX_BYTES) }),
  mismatch: () => m.password_mismatch(),
}
