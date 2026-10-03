import { useNavigate, useRouter } from "@tanstack/react-router"
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
import { formText } from "@/lib/form"
import { PASSWORD_MAX_BYTES } from "@/lib/password"
import { m } from "@/paraglide/messages.js"

import { signIn, type SignInOutcome } from "./sign-in"

type LoginReason =
  "expired" | "signed_out" | "signed_out_unconfirmed" | "password_reset"

const failures: Record<Exclude<SignInOutcome, "signed_in">, () => string> = {
  missing: () => m.login_missing(),
  invalid: () => m.login_invalid(),
  limited: () => m.failure_limited(),
  unavailable: () => m.login_unavailable(),
  clock: () => m.login_clock(),
}

const reasons: Record<
  LoginReason,
  { tone: "info" | "warning" | "success"; text: () => string }
> = {
  expired: { tone: "warning", text: () => m.login_expired() },
  signed_out: { tone: "info", text: () => m.login_signed_out() },
  signed_out_unconfirmed: {
    tone: "warning",
    text: () => m.login_signed_out_unconfirmed(),
  },
  password_reset: {
    tone: "success",
    text: () => m.entry_password_reset_done(),
  },
}

/**
 * A plain form (named fields, autocomplete, Enter submits) handled in the
 * page. Feedback goes to toasts, so the card never changes size.
 */
export function LoginCard({
  reason,
  returnTo,
}: {
  reason: LoginReason | undefined
  returnTo: string | undefined
}) {
  const navigate = useNavigate()
  const router = useRouter()
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
    setPending(true)
    const outcome = await signIn(
      formText(form, "email"),
      formText(form, "password")
    )
    if (outcome !== "signed_in") {
      setPending(false)
      toast.error(failures[outcome](), { id: "login" })
      if (outcome === "invalid") password.current?.select()
      return
    }
    toast.dismiss()
    setLeaving(true)
    // The card dissolves while the globe swings round to the network. The
    // way back is pushed verbatim: re-parsing its query would change values
    // such as a device request's `agent_version=1.0`.
    window.setTimeout(
      () =>
        returnTo ? router.history.push(returnTo) : void navigate({ to: "/" }),
      HANDOFF_MS
    )
  }

  return (
    <EntryCard title={m.login_title()} leaving={leaving}>
      <form
        method="post"
        action="/login"
        className="mt-7"
        onSubmit={(event) => void submit(event)}
      >
        <FieldGroup className="gap-5">
          <TextField
            size="lg"
            id="login-email"
            label={m.login_email()}
            placeholder={m.placeholder_email()}
            name="email"
            type="email"
            inputMode="email"
            autoComplete="username"
            autoCapitalize="none"
            spellCheck={false}
            autoFocus
            required
            maxLength={254}
          />
          <TextField
            size="lg"
            ref={password}
            id="login-password"
            label={m.login_password()}
            placeholder={m.placeholder_password()}
            name="password"
            type="password"
            autoComplete="current-password"
            required
            maxLength={PASSWORD_MAX_BYTES}
          />
          <EntrySubmit pending={pending} className="mt-2">
            {m.login_submit()}
          </EntrySubmit>
        </FieldGroup>
      </form>
      {/* A new instance has no account yet: its first owner is set up with the instance token. */}
      <EntryFoot>
        <span>{m.login_instance()}</span>
        <EntryLink
          disabled={pending || leaving}
          onClick={() => void navigate({ to: "/setup" })}
        >
          {m.login_setup()}
        </EntryLink>
      </EntryFoot>
    </EntryCard>
  )
}
