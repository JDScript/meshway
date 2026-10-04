import type { PasswordRules } from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { TextField } from "@/components/chrome/form"
import { FieldGroup } from "@/components/ui/field"
import { authOptionsQuery } from "@/features/auth/queries"
import { signIn, signUp } from "@/features/auth/sign-in"
import { failureText } from "@/lib/failure"
import { formText } from "@/lib/form"
import { passwordRulesText } from "@/lib/password-text"
import { m } from "@/paraglide/messages.js"

import { EntryFoot, EntryLink, EntrySubmit } from "./frame"
import { usePasswordFields } from "./password-fields"
import { codeOf } from "./session-call"

/** The card a sign-in happens in: its form action, its toast and field IDs, its words. */
export type InlineEntry = {
  action: string
  /** The card's toast ID, also the prefix of its field IDs. */
  toast: string
  signInPrompt: () => string
  registerPrompt: () => string
}

/**
 * Signing in (or registering, only while this controller takes
 * registrations) without leaving an entry card, so what the card holds
 * stays held.
 */
export function InlineSignIn({
  entry,
  onSignedIn,
}: {
  entry: InlineEntry
  onSignedIn: () => void
}) {
  const options = useQuery(authOptionsQuery())
  const [mode, setMode] = useState<"sign-in" | "register">("sign-in")
  const registration = options.data?.registrationEnabled
  const rules = options.data?.passwordRules ?? null
  const registering = mode === "register" && registration === true

  return (
    <>
      <p className="mt-2 text-row text-muted-foreground">
        {registering ? entry.registerPrompt() : entry.signInPrompt()}
      </p>
      {registering ? (
        <RegisterForm entry={entry} rules={rules} onSignedIn={onSignedIn} />
      ) : (
        <SignInForm entry={entry} onSignedIn={onSignedIn} />
      )}
      <EntryFoot>
        {registering ? (
          <>
            <span>{m.register_has_account()}</span>
            <EntryLink onClick={() => setMode("sign-in")}>
              {m.login_title()}
            </EntryLink>
          </>
        ) : registration === true ? (
          <>
            <span>{m.login_no_account()}</span>
            <EntryLink onClick={() => setMode("register")}>
              {m.register_title()}
            </EntryLink>
          </>
        ) : registration === false ? (
          <span>{m.register_closed()}</span>
        ) : null}
      </EntryFoot>
    </>
  )
}

/** What a sign-in or registration that did not start a session says. */
function announce(
  entry: InlineEntry,
  outcome: "signed_in" | "clock",
  onSignedIn: () => void
) {
  if (outcome === "clock") {
    toast.error(m.login_clock(), { id: entry.toast })
    return false
  }
  toast.dismiss(entry.toast)
  onSignedIn()
  return true
}

function SignInForm({
  entry,
  onSignedIn,
}: {
  entry: InlineEntry
  onSignedIn: () => void
}) {
  const [pending, setPending] = useState(false)
  const password = useRef<HTMLInputElement>(null)

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) return
    const form = new FormData(event.currentTarget)
    const email = formText(form, "email")
    const secret = formText(form, "password")
    if (!email.trim() || !secret) {
      toast.error(m.login_missing(), { id: entry.toast })
      return
    }
    setPending(true)
    try {
      if (!announce(entry, await signIn(email, secret), onSignedIn))
        setPending(false)
    } catch (error) {
      setPending(false)
      const text = failureText(error, {
        invalid_request: () => m.entry_email_invalid(),
      })
      if (text) toast.error(text, { id: entry.toast })
      if (codeOf(error) === "invalid_credentials") password.current?.select()
    }
  }

  return (
    <form
      method="post"
      action={entry.action}
      noValidate
      className="mt-7"
      onSubmit={(event) => void submit(event)}
    >
      <FieldGroup className="gap-5">
        <TextField
          size="lg"
          id={`${entry.toast}-email`}
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
          id={`${entry.toast}-password`}
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
  )
}

function RegisterForm({
  entry,
  rules,
  onSignedIn,
}: {
  entry: InlineEntry
  rules: PasswordRules | null
  onSignedIn: () => void
}) {
  const queryClient = useQueryClient()
  const [pending, setPending] = useState(false)
  const [emailInvalid, setEmailInvalid] = useState(false)
  const passwords = usePasswordFields()

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || !rules) return
    const form = new FormData(event.currentTarget)
    const email = formText(form, "email")
    const password = formText(form, "password")
    if (!email.trim()) {
      setEmailInvalid(true)
      toast.error(m.register_missing_email(), { id: entry.toast })
      return
    }
    setEmailInvalid(false)
    if (!passwords.check(rules, password, formText(form, "confirmation")))
      return
    setPending(true)
    try {
      if (!announce(entry, await signUp(email, password), onSignedIn))
        setPending(false)
    } catch (error) {
      setPending(false)
      const code = codeOf(error)
      if (code === "invalid_request" || code === "email_taken")
        setEmailInvalid(true)
      // Closed since the card read the options: it offers only the sign-in then.
      if (code === "registration_disabled")
        void queryClient.invalidateQueries({
          queryKey: authOptionsQuery().queryKey,
        })
      const text = failureText(error, {
        invalid_request: () => m.entry_email_invalid(),
      })
      if (text) toast.error(text, { id: entry.toast })
    }
  }

  return (
    <form
      method="post"
      action={entry.action}
      noValidate
      className="mt-7"
      onSubmit={(event) => void submit(event)}
    >
      <FieldGroup className="gap-5">
        <TextField
          size="lg"
          id={`${entry.toast}-register-email`}
          label={m.entry_email()}
          placeholder={m.placeholder_email()}
          name="email"
          type="text"
          inputMode="email"
          autoComplete="email"
          autoCapitalize="none"
          spellCheck={false}
          autoFocus
          invalid={emailInvalid}
          onChange={() => setEmailInvalid(false)}
        />
        <TextField
          size="lg"
          id={`${entry.toast}-register-password`}
          label={m.entry_password()}
          placeholder={
            rules ? passwordRulesText(rules) : m.placeholder_password_rules()
          }
          name="password"
          type="password"
          autoComplete="new-password"
          invalid={passwords.invalid === "password"}
          onChange={passwords.clear}
        />
        <TextField
          size="lg"
          id={`${entry.toast}-register-confirmation`}
          label={m.entry_confirm_password()}
          placeholder={m.placeholder_confirm_password()}
          name="confirmation"
          type="password"
          autoComplete="new-password"
          invalid={passwords.invalid === "confirmation"}
          onChange={passwords.clear}
        />
        <EntrySubmit pending={pending} disabled={!rules} className="mt-2">
          {m.register_submit()}
        </EntrySubmit>
      </FieldGroup>
    </form>
  )
}
