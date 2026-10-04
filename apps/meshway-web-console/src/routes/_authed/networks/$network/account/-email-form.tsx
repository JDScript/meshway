import { changeAccountEmail } from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { ApiFailure, authed, client, problemCode } from "@/api/client"
import { failureText } from "@/api/failure"
import { BusyLabel } from "@/components/chrome/busy-label"
import { TextField } from "@/components/chrome/form"
import { Section } from "@/components/chrome/section"
import { Button } from "@/components/ui/button"
import { formText } from "@/lib/form"
import { m } from "@/paraglide/messages.js"

const TOAST = "account-email"

type Field = "email" | "password"

/**
 * A new email, confirmed by the current password. The email goes as typed;
 * the controller normalises it, and answers with the one it keeps. No
 * session ends.
 */
export function EmailForm({ email }: { email: string | null }) {
  const queryClient = useQueryClient()
  const form = useRef<HTMLFormElement>(null)
  const [pending, setPending] = useState(false)
  const [invalid, setInvalid] = useState<Field | null>(null)

  const fail = (field: Field, text: string) => {
    setInvalid(field)
    toast.error(text, { id: TOAST })
    const input = form.current?.elements.namedItem(
      field === "email" ? "new-email" : "email-password"
    )
    if (input instanceof HTMLInputElement) input.select()
  }

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || email === null) return
    const data = new FormData(event.currentTarget)
    const next = formText(data, "new-email")
    const password = formText(data, "email-password")
    if (!next.trim()) return fail("email", m.account_email_missing())
    if (!password) return fail("password", m.account_password_missing())
    setInvalid(null)
    setPending(true)
    try {
      const changed = await authed((token) =>
        changeAccountEmail(client, { email: next, password }, { token })
      )
      form.current?.reset()
      if (changed.user.email === email)
        toast.info(m.account_email_unchanged(), { id: TOAST })
      else
        toast.success(m.account_email_changed({ email: changed.user.email }), {
          id: TOAST,
        })
      // The email shows across the console (the menu, member lists).
      void queryClient.invalidateQueries()
    } catch (error) {
      const code =
        error instanceof ApiFailure ? problemCode(error.failure) : null
      const text = failureText(error, {
        invalid_request: () => m.entry_email_invalid(),
        invalid_current_password: () => m.account_password_wrong(),
      })
      if (code === "invalid_current_password") fail("password", text ?? "")
      else if (code === "invalid_request" || code === "email_taken")
        fail("email", text ?? "")
      else if (text) toast.error(text, { id: TOAST })
    } finally {
      setPending(false)
    }
  }

  return (
    <Section title={m.account_email_section()}>
      <form
        ref={form}
        method="post"
        action="/account/email"
        noValidate
        aria-label={m.account_email_section()}
        className="flex flex-col gap-4"
        onSubmit={(event) => void submit(event)}
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
          id="account-new-email"
          label={m.account_email_new()}
          placeholder={m.placeholder_email()}
          name="new-email"
          type="text"
          inputMode="email"
          autoComplete="email"
          autoCapitalize="none"
          spellCheck={false}
          invalid={invalid === "email"}
          onChange={() => setInvalid(null)}
        />
        <TextField
          id="account-email-password"
          label={m.account_password_current()}
          placeholder={m.placeholder_password()}
          name="email-password"
          type="password"
          autoComplete="current-password"
          invalid={invalid === "password"}
          onChange={() => setInvalid(null)}
        />
        <div>
          <Button
            type="submit"
            className="h-9 rounded-full px-4"
            disabled={pending || email === null}
            aria-busy={pending}
          >
            <BusyLabel busy={pending}>{m.account_email_submit()}</BusyLabel>
          </Button>
        </div>
      </form>
    </Section>
  )
}
