import { useQuery } from "@tanstack/react-query"
import { useState } from "react"

import { authOptionsQuery } from "@/api/queries/auth"
import { m } from "@/paraglide/messages.js"

import { RegisterForm, SignInForm } from "./account-forms"
import { EntryFoot, EntryLink } from "./frame"

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
        <RegisterForm
          id={entry.toast}
          action={entry.action}
          rules={rules}
          onSignedIn={onSignedIn}
        />
      ) : (
        <SignInForm
          id={entry.toast}
          action={entry.action}
          onSignedIn={onSignedIn}
        />
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
