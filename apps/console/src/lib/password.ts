/** The controller's password bounds, in UTF-8 bytes. */
export const PASSWORD_MIN_BYTES = 12
export const PASSWORD_MAX_BYTES = 1024

export type PasswordProblem = "short" | "long" | "mismatch"

/** What is wrong with a new password and its confirmation, or null. */
export function newPasswordProblem(
  password: string,
  confirmation: string
): PasswordProblem | null {
  const bytes = new TextEncoder().encode(password).length
  if (bytes < PASSWORD_MIN_BYTES) return "short"
  if (bytes > PASSWORD_MAX_BYTES) return "long"
  if (password !== confirmation) return "mismatch"
  return null
}
