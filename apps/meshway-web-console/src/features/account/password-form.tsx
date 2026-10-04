import { changeAccountPassword } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { TextField } from "@/components/chrome/form"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { Section } from "@/components/chrome/section"
import { Button } from "@/components/ui/button"
import { authOptionsQuery } from "@/features/auth/queries"
import { useHandOver } from "@/features/auth/use-hand-over"
import { passwordRefusalText } from "@/features/entry/password-fields"
import { ApiFailure, authed, client, problemCode } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { formText } from "@/lib/form"
import { newPasswordProblem } from "@/lib/password"
import { passwordProblemText, passwordRulesText } from "@/lib/password-text"
import { m } from "@/paraglide/messages.js"

const TOAST = "account-password"

type Field = "current" | "next" | "confirm"

const FIELD_NAME: Record<Field, string> = {
  current: "current-password",
  next: "new-password",
  confirm: "confirm-password",
}

/**
 * The current password and a new one, checked against the rules the
 * controller publishes. Changing it ends every session of the account,
 * this tab's too, so it is confirmed first and then hands over to the
 * sign-in.
 */
export function PasswordForm({ email }: { email: string | null }) {
  const options = useQuery(authOptionsQuery())
  const rules = options.data?.passwordRules ?? null
  const handOver = useHandOver()
  const form = useRef<HTMLFormElement>(null)
  const [invalid, setInvalid] = useState<Field | null>(null)
  const [ready, setReady] = useState<{ current: string; next: string } | null>(
    null
  )
  const [confirming, setConfirming] = useState(false)
  const [pending, setPending] = useState(false)

  const fail = (field: Field, text: string) => {
    setInvalid(field)
    toast.error(text, { id: TOAST })
    const input = form.current?.elements.namedItem(FIELD_NAME[field])
    if (input instanceof HTMLInputElement) input.select()
  }

  const check = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || !rules || email === null) return
    const data = new FormData(event.currentTarget)
    const current = formText(data, FIELD_NAME.current)
    const next = formText(data, FIELD_NAME.next)
    if (!current) return fail("current", m.account_password_missing())
    const problem = newPasswordProblem(
      rules,
      next,
      formText(data, FIELD_NAME.confirm)
    )
    if (problem)
      return fail(
        problem === "mismatch" ? "confirm" : "next",
        passwordProblemText(problem, rules)
      )
    setInvalid(null)
    toast.dismiss(TOAST)
    setReady({ current, next })
    setConfirming(true)
  }

  const change = async () => {
    if (!ready || pending) return
    setPending(true)
    try {
      await authed((token) =>
        changeAccountPassword(
          client,
          { currentPassword: ready.current, password: ready.next },
          { token }
        )
      )
    } catch (error) {
      setPending(false)
      setConfirming(false)
      setReady(null)
      const code =
        error instanceof ApiFailure ? problemCode(error.failure) : null
      const text = failureText(error, {
        invalid_current_password: () => m.account_password_wrong(),
        password_weak: () => passwordRefusalText(error),
      })
      if (code === "invalid_current_password") fail("current", text ?? "")
      else if (code === "password_weak") fail("next", text ?? "")
      else if (text) toast.error(text, { id: TOAST })
      return
    }
    // Every session of the account, this one too, has ended.
    await handOver("password_changed")
  }

  return (
    <Section title={m.account_password_section()}>
      <form
        ref={form}
        method="post"
        action="/account/password"
        noValidate
        aria-label={m.account_password_section()}
        className="flex flex-col gap-4"
        onSubmit={check}
      >
        {/* For password managers: whose password this is. */}
        <input
          type="text"
          name="username"
          autoComplete="username"
          value={email ?? ""}
          readOnly
          hidden
        />
        <TextField
          id="account-current-password"
          label={m.account_password_current()}
          placeholder={m.placeholder_password()}
          name={FIELD_NAME.current}
          type="password"
          autoComplete="current-password"
          invalid={invalid === "current"}
          onChange={() => setInvalid(null)}
        />
        <TextField
          id="account-new-password"
          label={m.account_password_new()}
          placeholder={
            rules ? passwordRulesText(rules) : m.placeholder_password_rules()
          }
          name={FIELD_NAME.next}
          type="password"
          autoComplete="new-password"
          invalid={invalid === "next"}
          onChange={() => setInvalid(null)}
        />
        <TextField
          id="account-confirm-password"
          label={m.entry_confirm_password()}
          placeholder={m.placeholder_confirm_password()}
          name={FIELD_NAME.confirm}
          type="password"
          autoComplete="new-password"
          invalid={invalid === "confirm"}
          onChange={() => setInvalid(null)}
        />
        <div>
          <Button
            type="submit"
            className="h-9 rounded-full px-4"
            disabled={!rules || email === null || pending}
          >
            {m.account_password_submit()}
          </Button>
        </div>
      </form>
      <ConfirmDialog
        open={confirming}
        onOpenChange={(open) => {
          setConfirming(open)
          if (!open) setReady(null)
        }}
        title={m.account_password_submit()}
        confirmLabel={m.account_password_submit()}
        tone="default"
        pending={pending}
        onConfirm={() => void change()}
        effects={[
          m.account_password_effect_sessions(),
          m.account_password_effect_sign_in(),
        ]}
      />
    </Section>
  )
}
