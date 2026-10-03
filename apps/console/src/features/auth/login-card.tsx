import { useQuery } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useEffect, useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { TextField } from "@/components/chrome/form"
import { FieldGroup } from "@/components/ui/field"
import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntrySubmit,
  HANDOFF_MS,
} from "@/features/entry/frame"
import { ApiFailure, problemCode } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { formText } from "@/lib/form"
import { m } from "@/paraglide/messages.js"

import { authOptionsQuery } from "./queries"
import { signIn } from "./sign-in"

type LoginReason = "expired" | "signed_out" | "signed_out_unconfirmed"

const reasons: Record<
  LoginReason,
  { tone: "info" | "warning"; text: () => string }
> = {
  expired: { tone: "warning", text: () => m.login_expired() },
  signed_out: { tone: "info", text: () => m.login_signed_out() },
  signed_out_unconfirmed: {
    tone: "warning",
    text: () => m.login_signed_out_unconfirmed(),
  },
}

/**
 * A plain form (named fields, autocomplete, Enter submits) handled in the
 * page. Feedback goes to toasts, so the card never changes size.
 */
export function LoginCard({ reason }: { reason: LoginReason | undefined }) {
  const navigate = useNavigate()
  const [pending, setPending] = useState(false)
  const [leaving, setLeaving] = useState(false)
  const password = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (!reason) return
    const { tone, text } = reasons[reason]
    toast[tone](text(), { id: "login-reason" })
  }, [reason])

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) return
    const form = new FormData(event.currentTarget)
    const email = formText(form, "email")
    const secret = formText(form, "password")
    if (!email.trim() || !secret) {
      toast.error(m.login_missing(), { id: "login" })
      return
    }
    setPending(true)
    let outcome
    try {
      outcome = await signIn(email, secret)
    } catch (error) {
      setPending(false)
      const text = failureText(error, {
        invalid_request: () => m.entry_email_invalid(),
      })
      if (text) toast.error(text, { id: "login" })
      if (
        error instanceof ApiFailure &&
        problemCode(error.failure) === "invalid_credentials"
      )
        password.current?.select()
      return
    }
    if (outcome === "clock") {
      setPending(false)
      toast.error(m.login_clock(), { id: "login" })
      return
    }
    toast.dismiss()
    setLeaving(true)
    // The card dissolves before the signed-in page opens.
    window.setTimeout(() => void navigate({ to: "/" }), HANDOFF_MS)
  }

  return (
    <EntryCard title={m.login_title()} leaving={leaving}>
      <form
        method="post"
        action="/login"
        noValidate
        className="mt-7"
        onSubmit={(event) => void submit(event)}
      >
        <FieldGroup className="gap-5">
          <TextField
            size="lg"
            id="login-email"
            label={m.entry_email()}
            placeholder={m.placeholder_email()}
            name="email"
            type="text"
            inputMode="email"
            autoComplete="username"
            autoCapitalize="none"
            spellCheck={false}
            autoFocus
          />
          <TextField
            size="lg"
            ref={password}
            id="login-password"
            label={m.entry_password()}
            placeholder={m.placeholder_password()}
            name="password"
            type="password"
            autoComplete="current-password"
          />
          <EntrySubmit pending={pending} className="mt-2">
            {m.login_submit()}
          </EntrySubmit>
        </FieldGroup>
      </form>
      <RegistrationFoot
        disabled={pending || leaving}
        onRegister={() => void navigate({ to: "/register" })}
      />
    </EntryCard>
  )
}

/**
 * Whether this controller takes registrations, as it publishes. The line
 * keeps its height while the answer loads, or when it cannot be read.
 */
function RegistrationFoot({
  disabled,
  onRegister,
}: {
  disabled: boolean
  onRegister: () => void
}) {
  const options = useQuery(authOptionsQuery())
  const enabled = options.data?.registrationEnabled
  return (
    <EntryFoot>
      {enabled === true ? (
        <>
          <span>{m.login_no_account()}</span>
          <EntryLink disabled={disabled} onClick={onRegister}>
            {m.register_title()}
          </EntryLink>
        </>
      ) : enabled === false ? (
        <span>{m.register_closed()}</span>
      ) : null}
    </EntryFoot>
  )
}
