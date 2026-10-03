import {
  acceptAccountInvitation,
  ControlApiError,
  getAccountSession,
  registerInvitedAccount,
  type AccountMembership,
} from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useCallback, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { FieldGroup } from "@/components/ui/field"
import { Skeleton } from "@/components/ui/skeleton"
import { signIn, signOut } from "@/features/auth/sign-in"
import { networksQuery, sessionQuery } from "@/features/network/queries"
import { dropDrafts } from "@/features/shell/drafts"
import { apiBase } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { currentSession } from "@/lib/session"
import { TextField } from "@/components/chrome/form"
import { m } from "@/paraglide/messages.js"

import { EntryCard, EntryFoot, EntryLink, EntrySubmit } from "../frame"
import { heldInvitation } from "../held"
import { formText } from "@/lib/form"
import { usePasswordFields } from "../password-fields"
import { withSession } from "../session-call"

const RETURN_TO = "/invite"

const registerFailure = (error: unknown) =>
  failureText(error, {
    unauthorized: () => m.invite_rejected(),
    conflict: () => m.invite_email_taken(),
    invalid: () => m.invite_fields_rejected(),
  })

/**
 * An invitation is the only way to an account. Signed out: a new account
 * (its email must be the invited one). Signed in: this account joins.
 * Either way the browser then enters the invited network.
 */
export function InviteCard({ secret }: { secret: string }) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [signedIn, setSignedIn] = useState(() => currentSession() !== null)
  const [pending, setPending] = useState(false)
  const [leaving, setLeaving] = useState(false)

  const toLogin = useCallback(
    (reason?: "expired") => {
      queryClient.clear()
      void navigate({ to: "/login", search: { reason, returnTo: RETURN_TO } })
    },
    [navigate, queryClient]
  )
  const expired = useCallback(() => toLogin("expired"), [toLogin])

  const enter = (membership: AccountMembership) => {
    heldInvitation.drop()
    toast.dismiss()
    setLeaving(true)
    window.setTimeout(
      () =>
        void navigate({
          to: "/networks/$network/overview",
          params: { network: membership.network_id },
        }),
      320
    )
  }

  const switchAccount = async () => {
    await signOut()
    queryClient.clear()
    dropDrafts()
    setSignedIn(false)
  }

  return (
    <EntryCard title={m.invite_title()} leaving={leaving}>
      {signedIn ? (
        <AcceptForm
          secret={secret}
          pending={pending}
          setPending={setPending}
          onExpired={expired}
          onJoined={enter}
          onSwitch={() => void switchAccount()}
        />
      ) : (
        <RegisterForm
          secret={secret}
          pending={pending}
          setPending={setPending}
          onJoined={enter}
          onSignIn={() => toLogin()}
        />
      )}
    </EntryCard>
  )
}

type FormProps = {
  secret: string
  pending: boolean
  setPending: (pending: boolean) => void
  onJoined: (membership: AccountMembership) => void
}

function RegisterForm({
  secret,
  pending,
  setPending,
  onJoined,
  onSignIn,
}: FormProps & { onSignIn: () => void }) {
  const navigate = useNavigate()
  const password = usePasswordFields()

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) return
    const form = new FormData(event.currentTarget)
    const email = formText(form, "email").trim()
    const secretPassword = formText(form, "password")
    if (!password.check(secretPassword, formText(form, "confirmation"))) return
    setPending(true)
    let membership: AccountMembership
    try {
      membership = await registerInvitedAccount(apiBase(), {
        email,
        password: secretPassword,
        invitation_token: secret,
      })
    } catch (error) {
      setPending(false)
      toast.error(registerFailure(error), { id: "invite" })
      return
    }
    // The account exists now; signing in is a separate request.
    if ((await signIn(email, secretPassword)) === "signed_in") {
      onJoined(membership)
      return
    }
    heldInvitation.drop()
    toast.info(m.invite_created_sign_in(), { id: "invite" })
    await navigate({ to: "/login" })
  }

  return (
    <>
      <form
        method="post"
        action="/invite"
        className="mt-7"
        onSubmit={(event) => void submit(event)}
      >
        <FieldGroup className="gap-5">
          <TextField
            size="lg"
            id="invite-email"
            placeholder={m.placeholder_email()}
            label={m.entry_email()}
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
            id="invite-password"
            placeholder={m.placeholder_new_password()}
            label={m.entry_new_password()}
            name="password"
            type="password"
            autoComplete="new-password"
            required
            maxLength={1024}
            invalid={password.invalid === "password"}
            onInput={password.clear}
          />
          <TextField
            size="lg"
            id="invite-confirmation"
            placeholder={m.placeholder_confirm_password()}
            label={m.entry_confirm_password()}
            name="confirmation"
            type="password"
            autoComplete="new-password"
            required
            maxLength={1024}
            invalid={password.invalid === "confirmation"}
            onInput={password.clear}
          />
          <EntrySubmit pending={pending} className="mt-2">
            {m.invite_register()}
          </EntrySubmit>
        </FieldGroup>
      </form>
      <EntryFoot>
        <span>{m.invite_have_account()}</span>
        <EntryLink disabled={pending} onClick={onSignIn}>
          {m.invite_sign_in()}
        </EntryLink>
      </EntryFoot>
    </>
  )
}

function AcceptForm({
  secret,
  pending,
  setPending,
  onJoined,
  onExpired,
  onSwitch,
}: FormProps & { onExpired: () => void; onSwitch: () => void }) {
  const queryClient = useQueryClient()
  const session = useQuery({
    ...sessionQuery(),
    queryFn: () => withSession(getAccountSession, onExpired),
  })

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) return
    setPending(true)
    const signedIn = currentSession()
    if (!signedIn) {
      onExpired()
      return
    }
    let membership: AccountMembership
    try {
      membership = await acceptAccountInvitation(apiBase(), signedIn.token, {
        invitation_token: secret,
      })
    } catch (error) {
      setPending(false)
      if (error instanceof ControlApiError && error.status === 401) {
        // The invitation or the session: only a rejected session signs out.
        try {
          await withSession(getAccountSession, onExpired)
        } catch (check) {
          const text = failureText(check)
          if (text) toast.error(text, { id: "invite" })
          return
        }
        toast.error(m.invite_rejected(), { id: "invite" })
        return
      }
      const text = failureText(error, {
        conflict: () => m.invite_already_member(),
      })
      if (text) toast.error(text, { id: "invite" })
      return
    }
    await queryClient.invalidateQueries({ queryKey: sessionQuery().queryKey })
    await queryClient.invalidateQueries({ queryKey: networksQuery().queryKey })
    onJoined(membership)
  }

  const email = session.data?.user.email
  return (
    <>
      <form
        method="post"
        action="/invite"
        className="mt-7"
        onSubmit={(event) => void submit(event)}
      >
        <FieldGroup className="gap-5">
          <div className="flex h-11 items-center justify-between gap-4 rounded-tile bg-fill-subtle px-3.5 text-row">
            <span className="shrink-0 text-muted-foreground">
              {m.invite_account()}
            </span>
            {email ? (
              <span className="min-w-0 truncate font-medium" title={email}>
                {email}
              </span>
            ) : (
              <Skeleton className="h-3 w-36 rounded-full" />
            )}
          </div>
          <EntrySubmit pending={pending} className="mt-2" disabled={!email}>
            {m.invite_accept()}
          </EntrySubmit>
        </FieldGroup>
      </form>
      <EntryFoot>
        <span>{m.invite_other_account()}</span>
        <EntryLink disabled={pending} onClick={onSwitch}>
          {m.invite_sign_out()}
        </EntryLink>
      </EntryFoot>
    </>
  )
}
