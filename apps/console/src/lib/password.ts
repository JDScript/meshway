import type { PasswordRules } from "@meshway/control-client"

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
