import { useCallback, useState } from "react"
import { toast } from "sonner"

import { m } from "@/paraglide/messages.js"

import { newPasswordProblem, type PasswordProblem } from "./password"

const problemMessage: Record<PasswordProblem, () => string> = {
  short: () => m.entry_password_short(),
  long: () => m.entry_password_long(),
  mismatch: () => m.entry_password_mismatch(),
}

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
    toast.error(problemMessage[problem](), { id: "password" })
    return false
  }, [])
  const clear = useCallback(() => setInvalid(null), [])
  return { invalid, check, clear }
}
