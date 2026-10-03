import {
  revokeAccountSession,
  revokeAllAccountSessions,
  revokeOtherAccountSessions,
  type AccountSessionView,
} from "@meshway/control-client"
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query"
import { LogOutIcon, MonitorSmartphoneIcon } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { IconAction } from "@/components/chrome/icon-action"
import { RefList, RefRow, RefStateHeading } from "@/components/chrome/ref-row"
import { DangerRow, EmptyLine, Section } from "@/components/chrome/section"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useHandOver } from "@/features/auth/use-hand-over"
import { useSignOut } from "@/features/auth/use-sign-out"
import { useNow } from "@/hooks/use-now"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { authed, client } from "@/lib/api"
import { failureText, toastFailure } from "@/lib/failure"
import { dateTime, relativeTime } from "@/lib/format"
import { currentSession } from "@/lib/session"
import { m } from "@/paraglide/messages.js"

import { endsThisTab, pagedCount, shortId, type Revocation } from "./model"
import { accountActivityQuery, accountSessionsQuery } from "./queries"
import { useLoadMore } from "./use-load-more"

const TOAST = "account-sessions"

const revoke = (revocation: Revocation) =>
  authed((token) => {
    switch (revocation.kind) {
      case "one":
        return revokeAccountSession(
          client,
          { sessionId: revocation.sessionId },
          { token }
        )
      case "others":
        return revokeOtherAccountSessions(client, { token })
      case "all":
        return revokeAllAccountSessions(client, { token })
    }
  })

/**
 * The account's sessions that still hold, as the controller lists them;
 * this tab's is marked. What the controller does not know (the client, the
 * last use) is shown as unknown. Ending this tab's session hands over to
 * the sign-in.
 */
export function SessionList() {
  const sessions = useInfiniteQuery(accountSessionsQuery())
  const queryClient = useQueryClient()
  const handOver = useHandOver()
  const signOut = useSignOut()
  const loadMore = useLoadMore(sessions)
  const now = useNow(30_000)
  const [asked, setAsked] = useState<Revocation | null>(null)
  const [open, setOpen] = useState(false)
  const [pending, setPending] = useState(false)
  const [leaving, setLeaving] = useState(false)
  useReadFailureToast(
    "account-sessions",
    sessions.isError,
    failureText(sessions.error) ?? m.account_sessions_failed()
  )

  const rows = sessions.data?.pages.flatMap((page) => page.items) ?? []
  const unread = sessions.data === undefined
  const others = rows.some((row) => !row.current) || sessions.hasNextPage

  const ask = (revocation: Revocation) => {
    setAsked(revocation)
    setOpen(true)
  }

  const confirm = async () => {
    if (!asked || pending) return
    const session = currentSession()
    setPending(true)
    try {
      await revoke(asked)
    } catch (error) {
      setPending(false)
      setOpen(false)
      toastFailure(error)
      void queryClient.invalidateQueries({ queryKey: ["account", "sessions"] })
      return
    }
    if (session && endsThisTab(asked, session.id)) {
      setLeaving(true)
      await handOver(asked.kind === "all" ? "signed_out_all" : "signed_out")
      return
    }
    setPending(false)
    setOpen(false)
    toast.success(
      asked.kind === "others"
        ? m.account_revoked_others()
        : m.account_session_revoked(),
      { id: TOAST }
    )
    void queryClient.invalidateQueries({
      queryKey: accountSessionsQuery().queryKey,
    })
    void queryClient.invalidateQueries({
      queryKey: accountActivityQuery().queryKey,
    })
  }

  return (
    <Section
      title={m.account_sessions()}
      count={unread ? null : pagedCount(rows.length, sessions.hasNextPage)}
      action={<RefStateHeading>{m.account_session_expires()}</RefStateHeading>}
    >
      {unread ? (
        sessions.isError ? (
          <EmptyLine>{m.account_sessions_failed()}</EmptyLine>
        ) : (
          <Skeleton className="h-12 w-full rounded-tile" />
        )
      ) : (
        <RefList>
          {rows.map((row) => (
            <SessionRow
              key={row.id}
              session={row}
              now={now}
              disabled={leaving}
              onRevoke={() => ask({ kind: "one", sessionId: row.id })}
              onSignOut={() => {
                setLeaving(true)
                void signOut()
              }}
            />
          ))}
          {sessions.hasNextPage ? (
            <li>
              <Button
                variant="ghost"
                className="h-10 w-full justify-start rounded-tile px-3.5 text-muted-foreground"
                disabled={sessions.isFetchingNextPage}
                aria-busy={sessions.isFetchingNextPage}
                onClick={loadMore}
              >
                {m.account_sessions_more()}
              </Button>
            </li>
          ) : null}
        </RefList>
      )}
      <DangerRow
        fact={m.account_revoke_others_fact()}
        action={
          <Button
            variant="secondary"
            className="h-9 rounded-full px-4"
            disabled={unread || !others || leaving}
            onClick={() => ask({ kind: "others" })}
          >
            {m.account_revoke_others()}
          </Button>
        }
      />
      <DangerRow
        fact={m.account_revoke_all_fact()}
        action={
          <Button
            variant="destructive"
            className="h-9 rounded-full px-4"
            disabled={leaving}
            onClick={() => ask({ kind: "all" })}
          >
            {m.account_revoke_all()}
          </Button>
        }
      />
      <ConfirmDialog
        open={open}
        onOpenChange={(next) => {
          if (!pending) setOpen(next)
        }}
        title={revocationTitle(asked)}
        confirmLabel={revocationTitle(asked)}
        pending={pending}
        onConfirm={() => void confirm()}
        effects={revocationEffects(asked)}
      />
    </Section>
  )
}

const revocationTitle = (revocation: Revocation | null) => {
  switch (revocation?.kind) {
    case "others":
      return m.account_revoke_others()
    case "all":
      return m.account_revoke_all()
    default:
      return m.account_session_revoke()
  }
}

const revocationEffects = (revocation: Revocation | null) => {
  switch (revocation?.kind) {
    case "others":
      return [m.account_revoke_others_effect()]
    case "all":
      return [m.account_revoke_all_effect(), m.account_revoke_all_sign_in()]
    default:
      return [m.account_session_revoke_effect()]
  }
}

function SessionRow({
  session,
  now,
  disabled,
  onRevoke,
  onSignOut,
}: {
  session: AccountSessionView
  now: number
  disabled: boolean
  onRevoke: () => void
  onSignOut: () => void
}) {
  return (
    <RefRow
      lead={<MonitorSmartphoneIcon className="size-4 text-muted-foreground" />}
      title={
        <span className="flex min-w-0 items-center gap-1.5">
          <span className="truncate">
            {session.clientLabel ?? m.account_session_unknown_client()}
          </span>
          {session.current ? (
            <span className="shrink-0 rounded-full bg-fill-raised px-1.5 text-meta font-medium text-muted-foreground">
              {m.account_session_this()}
            </span>
          ) : null}
        </span>
      }
      subtitle={m.account_session_times({
        created: dateTime(session.createdAt) ?? m.account_unknown(),
        used:
          session.lastUsedAt === null
            ? m.account_unknown()
            : (relativeTime(session.lastUsedAt, now) ?? m.account_unknown()),
      })}
      detail={shortId(session.id)}
      state={relativeTime(session.expiresAt, now) ?? m.account_unknown()}
      action={
        session.current ? (
          <IconAction
            label={m.sign_out()}
            icon={LogOutIcon}
            disabled={disabled}
            onClick={onSignOut}
          />
        ) : (
          <IconAction
            label={m.account_session_revoke()}
            icon={LogOutIcon}
            disabled={disabled}
            onClick={onRevoke}
          />
        )
      }
    />
  )
}
