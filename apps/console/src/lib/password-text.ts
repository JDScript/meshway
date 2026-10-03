import type { PasswordRules } from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

import type { PasswordProblem } from "./password"

// Kept apart from the rules (`password.ts`) so those stay testable without the messages.

export function passwordProblemText(
  problem: PasswordProblem,
  rules: PasswordRules
): string {
  switch (problem) {
    case "short":
      return m.password_short({ min: String(rules.minCodepoints) })
    case "long":
      return m.password_long({ max: String(rules.maxCodepoints) })
    case "too_many_bytes":
      return m.password_too_many_bytes({ max: String(rules.maxUtf8Bytes) })
    case "mismatch":
      return m.password_mismatch()
  }
}

/** The published rules in one line, for the new password's field. */
export const passwordRulesText = (rules: PasswordRules) =>
  m.password_rules({
    min: String(rules.minCodepoints),
    max: String(rules.maxCodepoints),
  })
