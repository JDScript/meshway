import {
  getAuthSession,
  inspectJoinLink,
  redeemJoinLink,
  type JoinLinkInspection,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useCallback, useEffect, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { FieldGroup } from "@/components/ui/field"
import { Skeleton } from "@/components/ui/skeleton"
import { signOut } from "@/features/auth/sign-in"
import { networksQuery } from "@/features/network/queries"
import { roleLabel } from "@/features/network/roles"
import { client, SessionRejected } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { currentSession } from "@/lib/session"
import { m } from "@/paraglide/messages.js"

import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntrySubmit,
  EntryFact as Fact,
  HANDOFF_MS,
} from "../frame"
import { InlineSignIn, type InlineEntry } from "../inline-sign-in"
import { codeOf, withSession } from "../session-call"
import { JoinInvalid } from "./join-invalid"

const TOAST = "join"

const SIGN_IN: InlineEntry = {
  action: "/join",
  toast: TOAST,
  signInPrompt: () => m.join_sign_in_first(),
  registerPrompt: () => m.join_register_first(),
}

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
        <InlineSignIn entry={SIGN_IN} onSignedIn={() => setSignedIn(true)} />
      )}
    </EntryCard>
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
