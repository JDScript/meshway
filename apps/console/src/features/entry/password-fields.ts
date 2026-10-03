import { useCallback, useState } from "react"
import { toast } from "sonner"

import { newPasswordProblem } from "@/lib/password"
import { passwordProblemText } from "@/lib/password-text"

/**
 * A new password and its confirmation: checked before sending, the field at
 * fault marked in place (its ring) and the reason in a toast.
 */
export function usePasswordFields() {
  const [invalid, setInvalid] = useState<"password" | "confirmation" | null>(
    null
  )
  const check = useCallback((password: string, confirmation: string) => {
    const problem = newPasswordProblem(password, confirmation)
    if (!problem) {
      toast.dismiss("password")
      return true
    }
    setInvalid(problem === "mismatch" ? "confirmation" : "password")
    toast.error(passwordProblemText[problem](), { id: "password" })
    return false
  }, [])
  const clear = useCallback(() => setInvalid(null), [])
  return { invalid, check, clear }
}
