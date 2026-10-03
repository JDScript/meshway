import type { PasswordRules } from "@meshway/control-client"
import { useCallback, useState } from "react"
import { toast } from "sonner"

import { newPasswordProblem } from "@/lib/password"
import { passwordProblemText } from "@/lib/password-text"

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
