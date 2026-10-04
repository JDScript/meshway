import { useQuery } from "@tanstack/react-query"
import { createFileRoute, redirect, useNavigate } from "@tanstack/react-router"
import { useState, type ReactNode } from "react"
import { toast } from "sonner"

import { failureText } from "@/api/failure"
import { authOptionsQuery } from "@/api/queries/auth"
import { RegisterForm } from "@/components/entry/account-forms"
import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntryPage,
  HANDOFF_MS,
} from "@/components/entry/frame"
import { Button } from "@/components/ui/button"
import { m } from "@/paraglide/messages.js"
import { currentSession } from "@/state/session"

export const Route = createFileRoute("/register")({
  beforeLoad: () => {
    if (currentSession()) throw redirect({ to: "/" })
  },
  component: RegisterPage,
})

function RegisterPage() {
  return (
    <EntryPage>
      <RegisterCard />
    </EntryPage>
  )
}

/**
 * A new account, when this controller takes registrations. Its rules come
 * from the controller (`getAuthOptions`), which checks them again; closing
 * registration there closes it here, whatever this page last read.
 */
function RegisterCard() {
  const navigate = useNavigate()
  const options = useQuery(authOptionsQuery())
  const [leaving, setLeaving] = useState(false)
  const toLogin = () => void navigate({ to: "/login" })

  return (
    <EntryCard title={m.register_title()} leaving={leaving}>
      {options.data?.registrationEnabled ? (
        <RegisterForm
          id="register"
          action="/register"
          rules={options.data.passwordRules}
          onSignedIn={() => {
            toast.dismiss()
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
        <RegisterForm
          id="register"
          action="/register"
          rules={null}
          onSignedIn={() => undefined}
        />
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
