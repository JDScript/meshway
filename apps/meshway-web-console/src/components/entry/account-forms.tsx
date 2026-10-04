import type { PasswordRules } from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { failureText } from "@/api/failure"
import { authOptionsQuery } from "@/api/queries/auth"
import { codeOf } from "@/api/session-call"
import { signIn, signUp } from "@/api/sign-in"
import { TextField } from "@/components/chrome/form"
import { FieldGroup } from "@/components/ui/field"
import {
  passwordRefusalText,
  usePasswordFields,
} from "@/hooks/use-password-fields"
import { formText } from "@/lib/form"
import { passwordRulesText } from "@/lib/password"
import { m } from "@/paraglide/messages.js"

import { EntrySubmit } from "./frame"

// Signing in and registering, as plain forms (named fields, autocomplete,
// Enter submits) handled in the page: the sign-in page, the registration
// page, and the entry cards that sign in without leaving. Email and
// password go to the controller only; feedback goes to toasts, so a card
// never changes size.

type FormProps = {
  /** The card's toast ID, also the prefix of its field IDs. */
  id: string
  /** The form's action, for a browser that submits it without the console. */
  action: string
  /** A session started. */
  onSignedIn: () => void
}

/** What an issued session that cannot start says (its expiry already passed here). */
function started(id: string, outcome: "signed_in" | "clock") {
  if (outcome === "clock") {
    toast.error(m.login_clock(), { id })
    return false
  }
  toast.dismiss(id)
  return true
}

export function SignInForm({ id, action, onSignedIn }: FormProps) {
  const [pending, setPending] = useState(false)
  const password = useRef<HTMLInputElement>(null)

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) return
    const form = new FormData(event.currentTarget)
    const email = formText(form, "email")
    const secret = formText(form, "password")
    if (!email.trim() || !secret) {
      toast.error(m.login_missing(), { id })
      return
    }
    setPending(true)
    try {
      if (started(id, await signIn(email, secret))) onSignedIn()
      else setPending(false)
    } catch (error) {
      setPending(false)
      const text = failureText(error, {
        invalid_request: () => m.entry_email_invalid(),
      })
      if (text) toast.error(text, { id })
      if (codeOf(error) === "invalid_credentials") password.current?.select()
    }
  }

  return (
    <form
      method="post"
      action={action}
      noValidate
      className="mt-7"
      onSubmit={(event) => void submit(event)}
    >
      <FieldGroup className="gap-5">
        <TextField
          size="lg"
          id={`${id}-email`}
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
          id={`${id}-password`}
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

/**
 * A new account, by the password rules the controller publishes (it checks
 * them again); without them (still loading) the form shows but does not
 * submit. Registration closed since they were read shows it closed.
 */
export function RegisterForm({
  id,
  action,
  rules,
  onSignedIn,
}: FormProps & { rules: PasswordRules | null }) {
  const queryClient = useQueryClient()
  const [pending, setPending] = useState(false)
  const [emailInvalid, setEmailInvalid] = useState(false)
  const email = useRef<HTMLInputElement>(null)
  const passwords = usePasswordFields()

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || !rules) return
    const form = new FormData(event.currentTarget)
    const address = formText(form, "email")
    const password = formText(form, "password")
    if (!address.trim()) {
      setEmailInvalid(true)
      toast.error(m.register_missing_email(), { id })
      return
    }
    setEmailInvalid(false)
    if (!passwords.check(rules, password, formText(form, "confirmation")))
      return
    setPending(true)
    try {
      if (started(id, await signUp(address, password))) onSignedIn()
      else setPending(false)
    } catch (error) {
      setPending(false)
      const code = codeOf(error)
      if (code === "invalid_request" || code === "email_taken") {
        setEmailInvalid(true)
        email.current?.select()
      }
      if (code === "registration_disabled")
        void queryClient.invalidateQueries({
          queryKey: authOptionsQuery().queryKey,
        })
      const text = failureText(error, {
        invalid_request: () => m.entry_email_invalid(),
        password_weak: () => passwordRefusalText(error),
      })
      if (text) toast.error(text, { id })
    }
  }

  return (
    <form
      method="post"
      action={action}
      noValidate
      className="mt-7"
      onSubmit={(event) => void submit(event)}
    >
      <FieldGroup className="gap-5">
        <TextField
          size="lg"
          ref={email}
          id={`${id}-email`}
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
          id={`${id}-password`}
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
          id={`${id}-confirmation`}
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
