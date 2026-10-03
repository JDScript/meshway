import { redeemAccountPasswordReset } from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useState, type FormEvent } from "react"
import { toast } from "sonner"

import { FieldGroup } from "@/components/ui/field"
import { signOut } from "@/features/auth/sign-in"
import { dropDrafts } from "@/features/shell/drafts"
import { apiBase } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { currentSession } from "@/lib/session"
import { TextField } from "@/components/chrome/form"
import { m } from "@/paraglide/messages.js"

import { formText } from "@/lib/form"
import { EntryCard, EntrySubmit, HANDOFF_MS } from "../frame"
import { heldReset } from "../held"
import { usePasswordFields } from "../password-fields"

const redeemFailure = (error: unknown) =>
  failureText(error, {
    unauthorized: () => m.reset_rejected(),
    invalid: () => m.reset_password_rejected(),
  })

/**
 * A reset link from the instance administrator sets a new password. The
 * controller consumes the link and ends every session of the account.
 */
export function ResetCard({ secret }: { secret: string }) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const password = usePasswordFields()
  const [pending, setPending] = useState(false)
  const [leaving, setLeaving] = useState(false)

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) return
    const form = new FormData(event.currentTarget)
    const next = formText(form, "password")
    if (!password.check(next, formText(form, "confirmation"))) return
    setPending(true)
    try {
      await redeemAccountPasswordReset(apiBase(), {
        reset_token: secret,
        new_password: next,
      })
    } catch (error) {
      setPending(false)
      toast.error(redeemFailure(error), { id: "reset" })
      return
    }
    heldReset.drop()
    // The sign-in held by this tab may be one the reset has just ended.
    if (currentSession()) {
      await signOut()
      queryClient.clear()
      dropDrafts()
    }
    toast.dismiss()
    setLeaving(true)
    window.setTimeout(
      () =>
        void navigate({ to: "/login", search: { reason: "password_reset" } }),
      HANDOFF_MS
    )
  }

  return (
    <EntryCard title={m.reset_title()} leaving={leaving}>
      <form
        method="post"
        action="/reset-password"
        className="mt-7"
        onSubmit={(event) => void submit(event)}
      >
        <FieldGroup className="gap-5">
          <TextField
            size="lg"
            id="reset-password"
            placeholder={m.placeholder_new_password()}
            label={m.entry_new_password()}
            name="password"
            type="password"
            autoComplete="new-password"
            autoFocus
            required
            maxLength={1024}
            invalid={password.invalid === "password"}
            onInput={password.clear}
          />
          <TextField
            size="lg"
            id="reset-confirmation"
            placeholder={m.placeholder_confirm_password()}
            label={m.entry_confirm_password()}
            name="confirmation"
            type="password"
            autoComplete="new-password"
            required
            maxLength={1024}
            invalid={password.invalid === "confirmation"}
            onInput={password.clear}
          />
          <EntrySubmit pending={pending} className="mt-2">
            {m.reset_submit()}
          </EntrySubmit>
        </FieldGroup>
      </form>
    </EntryCard>
  )
}
