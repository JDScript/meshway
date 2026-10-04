import type { PasswordRules } from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

// The controller publishes its password rules (`getAuthOptions`) and
// measures them in Unicode code points and UTF-8 bytes; String.length
// counts UTF-16 units and is not used.

export type PasswordProblem = "short" | "long" | "too_many_bytes" | "mismatch"

export const codepoints = (text: string) => [...text].length
export const utf8Bytes = (text: string) => new TextEncoder().encode(text).length

/** What is wrong with a new password and its confirmation under `rules`, or null. */
export function newPasswordProblem(
  rules: PasswordRules,
  password: string,
  confirmation: string
): PasswordProblem | null {
  const length = codepoints(password)
  if (length < rules.minCodepoints) return "short"
  if (length > rules.maxCodepoints) return "long"
  if (utf8Bytes(password) > rules.maxUtf8Bytes) return "too_many_bytes"
  if (password !== confirmation) return "mismatch"
  return null
}

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
