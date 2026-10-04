import type { PasswordRules } from "@meshway/control-client"
import { useCallback, useState } from "react"
import { toast } from "sonner"

import { ApiFailure } from "@/lib/api"
import { newPasswordProblem } from "@/lib/password"
import { passwordProblemText } from "@/lib/password-text"
import { m } from "@/paraglide/messages.js"

/**
 * A new password and its confirmation: checked against the controller's
 * published rules before sending, the field at fault marked in place (its
 * ring) and the reason in a toast.
 */
export function usePasswordFields() {
  const [invalid, setInvalid] = useState<"password" | "confirmation" | null>(
    null
  )
  const check = useCallback(
    (rules: PasswordRules, password: string, confirmation: string) => {
      const problem = newPasswordProblem(rules, password, confirmation)
      if (!problem) {
        toast.dismiss("password")
        return true
      }
      setInvalid(problem === "mismatch" ? "confirmation" : "password")
      toast.error(passwordProblemText(problem, rules), { id: "password" })
      return false
    },
    []
  )
  const clear = useCallback(() => setInvalid(null), [])
  return { invalid, check, clear }
}

/** A `password_weak` refusal in the rules the controller sent with it, which may be newer than the page's. */
export function passwordRefusalText(error: unknown): string {
  const problem =
    error instanceof ApiFailure && error.failure.kind === "http"
      ? error.failure.problem?.problem
      : undefined
  if (problem?.code !== "password_weak") return m.problem_password_weak()
  return m.password_rules_unmet({
    min: String(problem.params.minCodepoints),
    max: String(problem.params.maxCodepoints),
    bytes: String(problem.params.maxUtf8Bytes),
  })
}
