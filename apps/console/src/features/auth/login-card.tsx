import { useNavigate } from "@tanstack/react-router"
import { useEffect, useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { Brand } from "@/components/chrome/brand"
import { Button } from "@/components/ui/button"
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { m } from "@/paraglide/messages.js"

import { signIn, type SignInOutcome } from "./sign-in"

export type LoginReason = "expired" | "signed_out" | "signed_out_unconfirmed"

const failures: Record<Exclude<SignInOutcome, "signed_in">, () => string> = {
  missing: () => m.login_missing(),
  invalid: () => m.login_invalid(),
  limited: () => m.login_limited(),
  unavailable: () => m.login_unavailable(),
  clock: () => m.login_clock(),
}

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
export function LoginCard({
  reason,
  returnTo,
}: {
  reason: LoginReason | undefined
  returnTo: string | undefined
}) {
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
    const email = form.get("email")
    const secret = form.get("password")
    setPending(true)
    const outcome = await signIn(
      typeof email === "string" ? email : "",
      typeof secret === "string" ? secret : ""
    )
    if (outcome !== "signed_in") {
      setPending(false)
      toast.error(failures[outcome](), { id: "login" })
      if (outcome === "invalid") password.current?.select()
      return
    }
    toast.dismiss()
    setLeaving(true)
    // The card dissolves while the globe swings round to the network.
    window.setTimeout(() => void navigate({ href: returnTo ?? "/" }), 320)
  }

  return (
    <div
      data-leaving={leaving}
      style={{
        "--enter-from": "16px",
        "--enter-scale": "0.985",
        "--enter-delay": "60ms",
      }}
      className="enter glass w-full max-w-95 rounded-sheet p-8 transition-[opacity,transform,filter] duration-500 ease-entrance data-[leaving=true]:-translate-y-3 data-[leaving=true]:scale-[0.98] data-[leaving=true]:opacity-0 data-[leaving=true]:blur-md motion-reduce:transition-none"
    >
      <Brand />
      <h1 className="mt-10 text-display font-semibold tracking-tight">
        {m.login_title()}
      </h1>
      <form
        method="post"
        action="/login"
        className="mt-7"
        onSubmit={(event) => void submit(event)}
      >
        <FieldGroup className="gap-5">
          <Field>
            <FieldLabel htmlFor="login-email">{m.login_email()}</FieldLabel>
            <Input
              id="login-email"
              name="email"
              type="email"
              inputMode="email"
              autoComplete="username"
              autoCapitalize="none"
              spellCheck={false}
              autoFocus
              required
              maxLength={254}
              className="h-11 rounded-tile bg-field px-3.5"
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="login-password">
              {m.login_password()}
            </FieldLabel>
            <Input
              ref={password}
              id="login-password"
              name="password"
              type="password"
              autoComplete="current-password"
              required
              maxLength={1024}
              className="h-11 rounded-tile bg-field px-3.5"
            />
          </Field>
          <Button
            type="submit"
            size="lg"
            className="mt-2 h-11 w-full rounded-tile"
            disabled={pending}
            aria-busy={pending}
          >
            <span className="relative inline-flex items-center justify-center">
              <Spinner
                className={
                  pending
                    ? "absolute -left-6 opacity-100"
                    : "absolute -left-6 opacity-0"
                }
              />
              {m.login_submit()}
            </span>
          </Button>
        </FieldGroup>
      </form>
    </div>
  )
}
