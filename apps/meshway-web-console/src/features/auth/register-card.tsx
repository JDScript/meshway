import type { PasswordRules } from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useRef, useState, type FormEvent, type ReactNode } from "react"
import { toast } from "sonner"

import { TextField } from "@/components/chrome/form"
import { Button } from "@/components/ui/button"
import { FieldGroup } from "@/components/ui/field"
import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntrySubmit,
  HANDOFF_MS,
} from "@/features/entry/frame"
import {
  passwordRefusalText,
  usePasswordFields,
} from "@/features/entry/password-fields"
import { ApiFailure, problemCode } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { formText } from "@/lib/form"
import { passwordRulesText } from "@/lib/password-text"
import { m } from "@/paraglide/messages.js"

import { authOptionsQuery } from "./queries"
import { signUp } from "./sign-in"

/**
 * A new account, when this controller takes registrations. Its rules come
 * from the controller (`getAuthOptions`), which checks them again; closing
 * registration there closes it here, whatever this page last read.
 */
export function RegisterCard() {
  const navigate = useNavigate()
  const options = useQuery(authOptionsQuery())
  const [leaving, setLeaving] = useState(false)
  const toLogin = () => void navigate({ to: "/login" })

  return (
    <EntryCard title={m.register_title()} leaving={leaving}>
      {options.data?.registrationEnabled ? (
        <RegisterForm
          rules={options.data.passwordRules}
          onDone={() => {
            setLeaving(true)
            window.setTimeout(() => void navigate({ to: "/" }), HANDOFF_MS)
          }}
        />
      ) : options.data ? (
        <Notice text={m.problem_registration_disabled()} />
      ) : options.isError ? (
        <Notice
          text={failureText(options.error) ?? m.failure_unknown()}
          action={
            <Button
              variant="outline"
              className="h-11 w-full rounded-tile"
              onClick={() => void options.refetch()}
            >
              {m.retry()}
            </Button>
          }
        />
      ) : (
        <RegisterForm rules={null} onDone={() => undefined} />
      )}
      <EntryFoot>
        <span>{m.register_has_account()}</span>
        <EntryLink disabled={leaving} onClick={toLogin}>
          {m.login_title()}
        </EntryLink>
      </EntryFoot>
    </EntryCard>
  )
}

function Notice({ text, action }: { text: string; action?: ReactNode }) {
  return (
    <div className="mt-7 flex flex-col gap-5">
      <p className="text-row text-muted-foreground">{text}</p>
      {action}
    </div>
  )
}

/** The form; without rules (still loading) it shows but does not submit. */
function RegisterForm({
  rules,
  onDone,
}: {
  rules: PasswordRules | null
  onDone: () => void
}) {
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
      toast.error(m.register_missing_email(), { id: "register" })
      return
    }
    setEmailInvalid(false)
    if (!passwords.check(rules, password, formText(form, "confirmation")))
      return
    setPending(true)
    let outcome
    try {
      outcome = await signUp(address, password)
    } catch (error) {
      setPending(false)
      const code =
        error instanceof ApiFailure ? problemCode(error.failure) : null
      if (code === "invalid_request" || code === "email_taken") {
        setEmailInvalid(true)
        email.current?.select()
      }
      // Closed since this page read the options: show it closed.
      if (code === "registration_disabled")
        void queryClient.invalidateQueries({
          queryKey: authOptionsQuery().queryKey,
        })
      const text = failureText(error, {
        invalid_request: () => m.entry_email_invalid(),
        password_weak: () => passwordRefusalText(error),
      })
      if (text) toast.error(text, { id: "register" })
      return
    }
    if (outcome === "clock") {
      setPending(false)
      toast.error(m.login_clock(), { id: "register" })
      return
    }
    toast.dismiss()
    onDone()
  }

  return (
    <form
      method="post"
      action="/register"
      noValidate
      className="mt-7"
      onSubmit={(event) => void submit(event)}
    >
      <FieldGroup className="gap-5">
        <TextField
          size="lg"
          ref={email}
          id="register-email"
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
          id="register-password"
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
          id="register-confirmation"
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
