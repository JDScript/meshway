import {
  inspectPasswordReset,
  redeemPasswordReset,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useEffect, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { call, client, codeOf } from "@/api/client"
import { failureText } from "@/api/failure"
import { authOptionsQuery } from "@/api/queries/auth"
import { signOut } from "@/api/sign-in"
import { TextField } from "@/components/chrome/form"
import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntrySubmit,
  HANDOFF_MS,
} from "@/components/entry/frame"
import { FieldGroup } from "@/components/ui/field"
import {
  passwordRefusalText,
  usePasswordFields,
} from "@/hooks/use-password-fields"
import { formText } from "@/lib/form"
import { dateTime } from "@/lib/format"
import { passwordRulesText } from "@/lib/password"
import { m } from "@/paraglide/messages.js"

const TOAST = "reset"

/** What the controller says of the link: checked once, before the form is offered. */
type ResetLink =
  | { state: "checking" }
  | { state: "valid"; expiresAt: string }
  | { state: "unusable"; text: string }
  | { state: "unread"; text: string }

/** Why the controller no longer takes the link, or null when it did not refuse it. */
function refusal(error: unknown): string | null {
  const code = codeOf(error)
  switch (code) {
    case "password_reset_used":
      return m.problem_password_reset_used()
    case "password_reset_expired":
      return m.problem_password_reset_expired()
    case "password_reset_replaced":
      return m.problem_password_reset_replaced()
    case "invalid_password_reset":
    case "invalid_request":
      return m.problem_invalid_password_reset()
    default:
      return null
  }
}

const NO_LINK = { state: "unusable", text: "" } as const

/** Inspects `secret` once; `retry` asks again after a failure that was not a refusal. */
function useResetLink(secret: string | null) {
  const [link, setLink] = useState<ResetLink>(
    secret ? { state: "checking" } : NO_LINK
  )
  const [attempt, setAttempt] = useState(0)
  useEffect(() => {
    if (!secret) return
    const abort = new AbortController()
    void call(
      inspectPasswordReset(client, { token: secret }, { signal: abort.signal })
    ).then(
      (inspection) =>
        setLink({ state: "valid", expiresAt: inspection.expiresAt }),
      (error: unknown) => {
        if (abort.signal.aborted) return
        const refused = refusal(error)
        setLink(
          refused !== null
            ? { state: "unusable", text: refused }
            : {
                state: "unread",
                text: failureText(error) ?? m.failure_unknown(),
              }
        )
      }
    )
    return () => abort.abort()
  }, [secret, attempt])
  const retry = () => {
    setLink({ state: "checking" })
    setAttempt((count) => count + 1)
  }
  return {
    link,
    retry,
    unusable: (text: string) => setLink({ state: "unusable", text }),
  }
}

/**
 * A reset link from the controller's host sets a new password. The
 * controller consumes the link and ends every session of the account; this
 * tab's own session, if it holds one, is ended too.
 */
export function ResetCard({
  secret,
  onUsed,
}: {
  secret: string | null
  /** The secret has been spent or refused: forget it. */
  onUsed: () => void
}) {
  const { link, retry, unusable } = useResetLink(secret)
  const navigate = useNavigate()
  const [leaving, setLeaving] = useState(false)
  const toLogin = () => void navigate({ to: "/login" })

  useEffect(() => {
    if (link.state === "unusable") onUsed()
  }, [link.state, onUsed])

  return (
    <EntryCard title={m.reset_title()} leaving={leaving}>
      {link.state === "unusable" ? (
        <p className="mt-7 text-row text-muted-foreground">
          {link.text || m.reset_unusable()}
        </p>
      ) : link.state === "unread" ? (
        <div className="mt-7 flex flex-col gap-5">
          <p className="text-row text-muted-foreground">{link.text}</p>
          <Button
            variant="outline"
            className="h-11 w-full rounded-tile"
            onClick={retry}
          >
            {m.retry()}
          </Button>
        </div>
      ) : (
        <ResetForm
          secret={secret ?? ""}
          expiresAt={link.state === "valid" ? link.expiresAt : null}
          onRefused={unusable}
          onDone={() => {
            onUsed()
            setLeaving(true)
            window.setTimeout(
              () =>
                void navigate({
                  to: "/login",
                  search: { reason: "password_reset" },
                }),
              HANDOFF_MS
            )
          }}
        />
      )}
      <EntryFoot>
        <span />
        <EntryLink disabled={leaving} onClick={toLogin}>
          {m.login_title()}
        </EntryLink>
      </EntryFoot>
    </EntryCard>
  )
}

/** The new password; until the link is checked (`expiresAt` null) it shows but does not submit. */
function ResetForm({
  secret,
  expiresAt,
  onRefused,
  onDone,
}: {
  secret: string
  expiresAt: string | null
  onRefused: (text: string) => void
  onDone: () => void
}) {
  const queryClient = useQueryClient()
  const options = useQuery(authOptionsQuery())
  const rules = options.data?.passwordRules ?? null
  const passwords = usePasswordFields()
  const [pending, setPending] = useState(false)

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || expiresAt === null || rules === null) return
    const form = new FormData(event.currentTarget)
    const password = formText(form, "password")
    if (!passwords.check(rules, password, formText(form, "confirmation")))
      return
    setPending(true)
    try {
      await call(redeemPasswordReset(client, { token: secret, password }))
    } catch (error) {
      setPending(false)
      const refused = refusal(error)
      if (refused !== null) {
        toast.error(refused, { id: TOAST })
        onRefused(refused)
        return
      }
      const text = failureText(error, {
        password_weak: () => passwordRefusalText(error),
      })
      if (text) toast.error(text, { id: TOAST })
      return
    }
    // The reset ended every session of that account; this tab's sign-in
    // may be one of them, or another account's: end it either way.
    await signOut()
    queryClient.clear()
    toast.dismiss()
    onDone()
  }

  return (
    <form
      method="post"
      action="/reset-password"
      noValidate
      className="mt-7"
      onSubmit={(event) => void submit(event)}
    >
      <p className="mb-5 flex h-5 items-center text-row text-muted-foreground">
        {expiresAt
          ? m.reset_expires({ time: dateTime(expiresAt) ?? "—" })
          : m.reset_checking()}
      </p>
      <FieldGroup className="gap-5">
        <TextField
          size="lg"
          id="reset-password"
          label={m.entry_new_password()}
          placeholder={
            rules ? passwordRulesText(rules) : m.placeholder_password_rules()
          }
          name="password"
          type="password"
          autoComplete="new-password"
          autoFocus
          invalid={passwords.invalid === "password"}
          onChange={passwords.clear}
        />
        <TextField
          size="lg"
          id="reset-confirmation"
          label={m.entry_confirm_password()}
          placeholder={m.placeholder_confirm_password()}
          name="confirmation"
          type="password"
          autoComplete="new-password"
          invalid={passwords.invalid === "confirmation"}
          onChange={passwords.clear}
        />
        <EntrySubmit
          pending={pending}
          disabled={expiresAt === null || rules === null}
          className="mt-2"
        >
          {m.reset_submit()}
        </EntrySubmit>
      </FieldGroup>
    </form>
  )
}
