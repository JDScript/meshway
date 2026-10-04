import { useQuery } from "@tanstack/react-query"
import { createFileRoute, redirect, useNavigate } from "@tanstack/react-router"
import { useEffect, useState } from "react"
import { toast } from "sonner"
import { z } from "zod"

import { authOptionsQuery } from "@/api/queries/auth"
import { SignInForm } from "@/components/entry/account-forms"
import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntryPage,
  HANDOFF_MS,
} from "@/components/entry/frame"
import { LOGIN_REASONS, type LoginReason } from "@/lib/login-reasons"
import { m } from "@/paraglide/messages.js"
import { currentSession } from "@/state/session"

const zSearch = z.object({
  reason: z.enum(LOGIN_REASONS).optional().catch(undefined),
})

export const Route = createFileRoute("/login")({
  validateSearch: zSearch,
  beforeLoad: () => {
    if (currentSession()) throw redirect({ to: "/" })
  },
  component: LoginPage,
})

function LoginPage() {
  const { reason } = Route.useSearch()
  return (
    <EntryPage>
      <LoginCard reason={reason} />
    </EntryPage>
  )
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
  signed_out_all: { tone: "info", text: () => m.login_signed_out_all() },
  password_changed: { tone: "info", text: () => m.login_password_changed() },
  password_reset: { tone: "info", text: () => m.login_password_reset() },
}

/** The sign-in page's card; a reason in the address says why it opened. */
function LoginCard({ reason }: { reason: LoginReason | undefined }) {
  const navigate = useNavigate()
  const [leaving, setLeaving] = useState(false)

  useEffect(() => {
    if (!reason) return
    const { tone, text } = reasons[reason]
    toast[tone](text(), { id: "login-reason" })
  }, [reason])

  const signedIn = () => {
    toast.dismiss()
    setLeaving(true)
    // The card dissolves before the signed-in page opens.
    window.setTimeout(() => void navigate({ to: "/" }), HANDOFF_MS)
  }

  return (
    <EntryCard title={m.login_title()} leaving={leaving}>
      <SignInForm id="login" action="/login" onSignedIn={signedIn} />
      <RegistrationFoot
        disabled={leaving}
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
