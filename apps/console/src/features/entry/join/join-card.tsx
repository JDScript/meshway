import {
  getAuthSession,
  inspectJoinLink,
  redeemJoinLink,
  type ClientResult,
  type ErrorResponse,
  type JoinLinkInspection,
  type PasswordRules,
} from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react"
import { toast } from "sonner"

import { TextField } from "@/components/chrome/form"
import { Button } from "@/components/ui/button"
import { FieldGroup } from "@/components/ui/field"
import { Skeleton } from "@/components/ui/skeleton"
import { authOptionsQuery } from "@/features/auth/queries"
import { signIn, signOut, signUp } from "@/features/auth/sign-in"
import { networksQuery } from "@/features/network/queries"
import { roleLabel } from "@/features/network/roles"
import { ApiFailure, client, problemCode, SessionRejected } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { formText } from "@/lib/form"
import { dateTime } from "@/lib/format"
import { passwordRulesText } from "@/lib/password-text"
import { currentSession, endSession } from "@/lib/session"
import { m } from "@/paraglide/messages.js"

import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntrySubmit,
  HANDOFF_MS,
} from "../frame"
import { usePasswordFields } from "../password-fields"
import { JoinInvalid } from "./join-invalid"

const TOAST = "join"

/**
 * A call with this tab's session that keeps the page: a rejected session
 * ends here and the card asks for a sign-in again, so the held link is not
 * lost to the sign-in page.
 */
async function withSession<T>(
  operation: (token: string) => Promise<ClientResult<T, ErrorResponse>>
): Promise<T> {
  const session = currentSession()
  if (!session) throw new SessionRejected()
  const settled = await operation(session.token)
  if (settled.ok) return settled.data
  if (settled.error.kind === "http" && settled.error.status === 401) {
    endSession(session)
    throw new SessionRejected()
  }
  throw new ApiFailure(settled.error)
}

const codeOf = (error: unknown) =>
  error instanceof ApiFailure ? problemCode(error.failure) : null

/**
 * A join link: an existing account accepts it into the network and role it
 * names. Signed out, the card signs in (or registers, only while this
 * controller takes registrations) in place, keeping the link; signed in, it
 * shows what the link is for and the account that would join.
 */
export function JoinCard({
  secret,
  onDrop,
}: {
  secret: string
  /** The link is used up or left behind: forget it. */
  onDrop: () => void
}) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [signedIn, setSignedIn] = useState(() => currentSession() !== null)
  const [invalid, setInvalid] = useState(false)
  const [leaving, setLeaving] = useState(false)

  const expired = useCallback(() => {
    queryClient.clear()
    setSignedIn(false)
    toast.warning(m.login_expired(), { id: TOAST })
  }, [queryClient])

  const markInvalid = useCallback(() => {
    onDrop()
    setInvalid(true)
  }, [onDrop])

  const open = (networkId: string) => {
    onDrop()
    setLeaving(true)
    window.setTimeout(
      () =>
        void navigate({
          to: "/networks/$network/overview",
          params: { network: networkId },
        }),
      HANDOFF_MS
    )
  }

  if (invalid) return <JoinInvalid />

  return (
    <EntryCard title={m.join_title()} leaving={leaving}>
      {signedIn ? (
        <Accept
          secret={secret}
          leaving={leaving}
          onExpired={expired}
          onInvalid={markInvalid}
          onJoined={open}
          onSignOut={async () => {
            await signOut()
            queryClient.clear()
            setSignedIn(false)
          }}
        />
      ) : (
        <SignIn onSignedIn={() => setSignedIn(true)} />
      )}
    </EntryCard>
  )
}

/** One fact about the link, on a tile the height of a field. */
function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex h-11 items-center justify-between gap-4 rounded-tile bg-fill-subtle px-3.5 text-row">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="flex min-w-0 justify-end truncate font-medium">
        {children}
      </span>
    </div>
  )
}

const unread = <Skeleton className="h-3 w-28 rounded-full" />

function Accept({
  secret,
  leaving,
  onExpired,
  onInvalid,
  onJoined,
  onSignOut,
}: {
  secret: string
  leaving: boolean
  onExpired: () => void
  onInvalid: () => void
  onJoined: (networkId: string) => void
  onSignOut: () => Promise<void>
}) {
  const queryClient = useQueryClient()
  const [email, setEmail] = useState<string | null>(null)
  const [inspection, setInspection] = useState<JoinLinkInspection | null>(null)
  const [failed, setFailed] = useState(false)
  const [attempt, setAttempt] = useState(0)
  const [pending, setPending] = useState(false)

  // The link is read, never cached: its secret is in no query key or cache.
  useEffect(() => {
    let live = true
    const settle = (error: unknown) => {
      if (!live) return
      if (error instanceof SessionRejected) return onExpired()
      if (codeOf(error) === "invalid_join_link") return onInvalid()
      setFailed(true)
      const text = failureText(error, {
        invalid_request: () => m.problem_invalid_join_link(),
      })
      if (text) toast.error(text, { id: TOAST })
    }
    Promise.all([
      withSession((token) => getAuthSession(client, { token })),
      withSession((token) =>
        inspectJoinLink(client, { token: secret }, { token })
      ),
    ]).then(([session, inspected]) => {
      if (!live) return
      setEmail(session.user.email)
      setInspection(inspected)
    }, settle)
    return () => {
      live = false
    }
  }, [secret, attempt, onExpired, onInvalid])

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || !inspection) return
    setPending(true)
    try {
      const joined = await withSession((token) =>
        redeemJoinLink(client, { token: secret }, { token })
      )
      await queryClient.invalidateQueries({
        queryKey: networksQuery().queryKey,
      })
      toast.dismiss(TOAST)
      onJoined(joined.networkId)
    } catch (error) {
      setPending(false)
      if (error instanceof SessionRejected) return onExpired()
      const code = codeOf(error)
      if (code === "already_member") {
        toast.info(m.join_already_member(), { id: TOAST })
        onJoined(inspection.link.networkId)
        return
      }
      if (code === "invalid_join_link") return onInvalid()
      const text = failureText(error)
      if (text) toast.error(text, { id: TOAST })
    }
  }

  const link = inspection?.link
  return (
    <>
      <form
        method="post"
        action="/join"
        className="mt-7"
        onSubmit={(event) => void submit(event)}
      >
        <FieldGroup className="gap-2">
          <Fact label={m.join_network()}>
            {inspection ? (
              <span className="truncate" title={inspection.networkName}>
                {inspection.networkName}
              </span>
            ) : (
              unread
            )}
          </Fact>
          <Fact label={m.join_role()}>
            {link ? roleLabel[link.role]() : unread}
          </Fact>
          <Fact label={m.join_expires()}>
            {link ? (dateTime(link.expiresAt) ?? "—") : unread}
          </Fact>
          <Fact label={m.join_account()}>
            {email ? (
              <span className="truncate" title={email}>
                {email}
              </span>
            ) : (
              unread
            )}
          </Fact>
          {failed ? (
            <Button
              type="button"
              variant="outline"
              size="lg"
              className="mt-5 h-11 w-full rounded-tile"
              onClick={() => {
                setFailed(false)
                setAttempt((count) => count + 1)
              }}
            >
              {m.retry()}
            </Button>
          ) : (
            <EntrySubmit
              pending={pending}
              disabled={!inspection || leaving}
              className="mt-5"
            >
              {m.join_accept()}
            </EntrySubmit>
          )}
        </FieldGroup>
      </form>
      <EntryFoot>
        <span>{m.join_other_account()}</span>
        <EntryLink
          disabled={pending || leaving}
          onClick={() => void onSignOut()}
        >
          {m.sign_out()}
        </EntryLink>
      </EntryFoot>
    </>
  )
}

/** Signing in (or registering) without leaving the card, so the link stays held. */
function SignIn({ onSignedIn }: { onSignedIn: () => void }) {
  const options = useQuery(authOptionsQuery())
  const [mode, setMode] = useState<"sign-in" | "register">("sign-in")
  const registration = options.data?.registrationEnabled
  const rules = options.data?.passwordRules ?? null
  const registering = mode === "register" && registration === true

  return (
    <>
      <p className="mt-2 text-row text-muted-foreground">
        {registering ? m.join_register_first() : m.join_sign_in_first()}
      </p>
      {registering ? (
        <RegisterForm rules={rules} onSignedIn={onSignedIn} />
      ) : (
        <SignInForm onSignedIn={onSignedIn} />
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

/** What a sign-in or registration that did not start a session says. */
function announce(outcome: "signed_in" | "clock", onSignedIn: () => void) {
  if (outcome === "clock") {
    toast.error(m.login_clock(), { id: TOAST })
    return false
  }
  toast.dismiss(TOAST)
  onSignedIn()
  return true
}

function SignInForm({ onSignedIn }: { onSignedIn: () => void }) {
  const [pending, setPending] = useState(false)
  const password = useRef<HTMLInputElement>(null)

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) return
    const form = new FormData(event.currentTarget)
    const email = formText(form, "email")
    const secret = formText(form, "password")
    if (!email.trim() || !secret) {
      toast.error(m.login_missing(), { id: TOAST })
      return
    }
    setPending(true)
    try {
      if (!announce(await signIn(email, secret), onSignedIn)) setPending(false)
    } catch (error) {
      setPending(false)
      const text = failureText(error, {
        invalid_request: () => m.entry_email_invalid(),
      })
      if (text) toast.error(text, { id: TOAST })
      if (codeOf(error) === "invalid_credentials") password.current?.select()
    }
  }

  return (
    <form
      method="post"
      action="/join"
      noValidate
      className="mt-7"
      onSubmit={(event) => void submit(event)}
    >
      <FieldGroup className="gap-5">
        <TextField
          size="lg"
          id="join-email"
          label={m.entry_email()}
          placeholder={m.placeholder_email()}
          name="email"
          type="text"
          inputMode="email"
          autoComplete="username"
          autoCapitalize="none"
          spellCheck={false}
          autoFocus
        />
        <TextField
          size="lg"
          ref={password}
          id="join-password"
          label={m.entry_password()}
          placeholder={m.placeholder_password()}
          name="password"
          type="password"
          autoComplete="current-password"
        />
        <EntrySubmit pending={pending} className="mt-2">
          {m.login_submit()}
        </EntrySubmit>
      </FieldGroup>
    </form>
  )
}

function RegisterForm({
  rules,
  onSignedIn,
}: {
  rules: PasswordRules | null
  onSignedIn: () => void
}) {
  const queryClient = useQueryClient()
  const [pending, setPending] = useState(false)
  const [emailInvalid, setEmailInvalid] = useState(false)
  const passwords = usePasswordFields()

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || !rules) return
    const form = new FormData(event.currentTarget)
    const email = formText(form, "email")
    const password = formText(form, "password")
    if (!email.trim()) {
      setEmailInvalid(true)
      toast.error(m.register_missing_email(), { id: TOAST })
      return
    }
    setEmailInvalid(false)
    if (!passwords.check(rules, password, formText(form, "confirmation")))
      return
    setPending(true)
    try {
      if (!announce(await signUp(email, password), onSignedIn))
        setPending(false)
    } catch (error) {
      setPending(false)
      const code = codeOf(error)
      if (code === "invalid_request" || code === "email_taken")
        setEmailInvalid(true)
      // Closed since the card read the options: it offers only the sign-in then.
      if (code === "registration_disabled")
        void queryClient.invalidateQueries({
          queryKey: authOptionsQuery().queryKey,
        })
      const text = failureText(error, {
        invalid_request: () => m.entry_email_invalid(),
      })
      if (text) toast.error(text, { id: TOAST })
    }
  }

  return (
    <form
      method="post"
      action="/join"
      noValidate
      className="mt-7"
      onSubmit={(event) => void submit(event)}
    >
      <FieldGroup className="gap-5">
        <TextField
          size="lg"
          id="join-register-email"
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
          id="join-register-password"
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
          id="join-register-confirmation"
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
