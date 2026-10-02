import {
  ControlApiError,
  redeemAccountPasswordReset,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useState, type FormEvent } from "react"
import { toast } from "sonner"

import { FieldGroup } from "@/components/ui/field"
import { signOut } from "@/features/auth/sign-in"
import { dropDrafts } from "@/features/shell/drafts"
import { apiBase } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import { currentSession } from "@/lib/session"
import { TextField } from "@/components/chrome/form"
import { m } from "@/paraglide/messages.js"

import { formText } from "../form"
import { EntryCard, EntrySubmit } from "../frame"
import { heldReset } from "../held"
import { usePasswordFields } from "../password-fields"

function redeemFailure(error: unknown): string {
  if (error instanceof ControlApiError) {
    if (error.status === 401) return m.reset_rejected()
    if (error.status === 429) return m.entry_limited()
    if (error.status === 400 || error.status === 422)
      return m.reset_password_rejected()
  }
  return failureMessage.unknown()
}

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
      320
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
