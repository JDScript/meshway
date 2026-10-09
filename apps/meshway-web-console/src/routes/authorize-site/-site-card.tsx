import { createDelegation, getAuthSession } from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useQueryClient } from "@tanstack/react-query"
import { useCallback, useEffect, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { client, SessionRejected } from "@/api/client"
import { failureText } from "@/api/failure"
import { withSession } from "@/api/session-call"
import { signOut } from "@/api/sign-in"
import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntrySubmit,
  EntryFact as Fact,
} from "@/components/entry/frame"
import {
  InlineSignIn,
  type InlineEntry,
} from "@/components/entry/inline-sign-in"
import { FieldGroup } from "@/components/ui/field"
import { Skeleton } from "@/components/ui/skeleton"
import {
  delegationClientName,
  returnAddress,
  type SiteAuthorization,
} from "@/lib/delegation"
import { m } from "@/paraglide/messages.js"
import { currentSession } from "@/state/session"

const TOAST = "site"

const SIGN_IN: InlineEntry = {
  action: "/authorize-site",
  toast: TOAST,
  signInPrompt: () => m.site_sign_in_first(),
  registerPrompt: () => m.site_register_first(),
}

/**
 * A site's request to act for this account. Signed out, the card signs in
 * (or registers, only while registration is open) in place. Consent has
 * the server issue a one-time code bound to this sign-in and returns to the
 * site with it; the server decides whether the site and its address are
 * allowed. Refusing returns nothing anywhere: the address is the site's
 * claim until the server has checked it.
 */
export function SiteCard({ request }: { request: SiteAuthorization | null }) {
  const queryClient = useQueryClient()
  const [signedIn, setSignedIn] = useState(() => currentSession() !== null)

  const expired = useCallback(() => {
    queryClient.clear()
    setSignedIn(false)
    toast.warning(m.login_expired(), { id: TOAST })
  }, [queryClient])

  if (request === null)
    return (
      <EntryCard title={m.site_invalid()}>
        <p className="mt-7 text-row text-muted-foreground">
          {m.site_invalid_text()}
        </p>
      </EntryCard>
    )

  return (
    <EntryCard title={m.site_title()}>
      {signedIn ? (
        <Consent
          request={request}
          onExpired={expired}
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

function Consent({
  request,
  onExpired,
  onSignOut,
}: {
  request: SiteAuthorization
  onExpired: () => void
  onSignOut: () => Promise<void>
}) {
  const [email, setEmail] = useState<string | null>(null)
  /** The account read failed: why, until asked again. */
  const [unread, setUnread] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  const [pending, setPending] = useState(false)
  const [denied, setDenied] = useState(false)

  useEffect(() => {
    let live = true
    withSession((token) => getAuthSession(client, { token })).then(
      (session) => {
        if (live) setEmail(session.user.email)
      },
      (error: unknown) => {
        if (!live) return
        if (error instanceof SessionRejected) return onExpired()
        setUnread(failureText(error) ?? m.site_account_failed())
      }
    )
    return () => {
      live = false
    }
  }, [onExpired, attempt])

  const allow = async () => {
    if (pending) return
    setPending(true)
    try {
      const issued = await withSession((token) =>
        createDelegation(
          client,
          {
            client: request.client,
            redirectUri: request.redirectUri.href,
            codeChallenge: request.codeChallenge,
            codeChallengeMethod: "S256",
          },
          { token }
        )
      )
      // Replaced, so going back does not land on a used consent.
      window.location.replace(returnAddress(request, issued.code))
    } catch (error) {
      setPending(false)
      if (error instanceof SessionRejected) return onExpired()
      const text = failureText(error, {
        invalid_request: () => m.site_refused(),
        permission_denied: () => m.site_forbidden(),
      })
      if (text) toast.error(text, { id: TOAST })
    }
  }

  const name = delegationClientName(request.client)

  return (
    <>
      <form
        method="post"
        action="/authorize-site"
        className="mt-7"
        onSubmit={(event: FormEvent<HTMLFormElement>) => {
          event.preventDefault()
          void allow()
        }}
      >
        <FieldGroup className="gap-2">
          <Fact label={m.site_client()}>
            <span className="truncate">{name}</span>
          </Fact>
          <Fact label={m.site_returns_to()}>
            <span
              className="truncate font-mono"
              title={request.redirectUri.origin}
            >
              {request.redirectUri.host}
            </span>
          </Fact>
          <Fact label={m.site_lifetime()}>{m.site_lifetime_value()}</Fact>
          <Fact label={m.site_account()}>
            {email ? (
              <span className="truncate" title={email}>
                {email}
              </span>
            ) : unread !== null ? (
              <span className="flex min-w-0 items-center gap-2 text-muted-foreground">
                <span className="truncate">{unread}</span>
                <EntryLink
                  onClick={() => {
                    setUnread(null)
                    setAttempt((value) => value + 1)
                  }}
                >
                  {m.retry()}
                </EntryLink>
              </span>
            ) : (
              <Skeleton className="h-3 w-28 rounded-full" />
            )}
          </Fact>
          <p className="mt-1 text-row text-muted-foreground">
            {m.site_scope({ name })}
          </p>
          {denied ? (
            <div className="mt-3 flex flex-col gap-4">
              <p role="status" className="text-row text-muted-foreground">
                {m.site_denied_text()}
              </p>
              <Button
                type="button"
                variant="outline"
                size="lg"
                className="h-11 w-full rounded-tile"
                onClick={() => window.history.back()}
              >
                {m.site_back()}
              </Button>
            </div>
          ) : (
            // Allow first in the form, so Enter allows; shown on the right.
            <div className="mt-3 flex flex-row-reverse gap-3">
              <EntrySubmit
                className="flex-1"
                pending={pending}
                disabled={pending || email === null}
              >
                {m.site_allow()}
              </EntrySubmit>
              <Button
                type="button"
                variant="outline"
                size="lg"
                className="h-11 flex-1 rounded-tile"
                disabled={pending}
                onClick={() => setDenied(true)}
              >
                {m.site_deny()}
              </Button>
            </div>
          )}
        </FieldGroup>
      </form>
      <EntryFoot>
        <span>{m.device_other_account()}</span>
        <EntryLink disabled={pending} onClick={() => void onSignOut()}>
          {m.sign_out()}
        </EntryLink>
      </EntryFoot>
    </>
  )
}
