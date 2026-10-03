import { useQuery } from "@tanstack/react-query"
import { useState } from "react"

import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { sessionQuery } from "@/features/auth/queries"
import { useSignOut } from "@/features/auth/use-sign-out"
import { EntryCard } from "@/features/entry/frame"
import { failureText } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

const BUTTON = "h-11 w-full rounded-tile"

/**
 * The signed-in account, as the controller reports this tab's session. This
 * controller has no network operations yet, and the card says so rather
 * than showing an empty list.
 */
export function AccountCard() {
  const session = useQuery(sessionQuery())
  const signOut = useSignOut()
  const [leaving, setLeaving] = useState(false)

  const leave = () => {
    setLeaving(true)
    void signOut()
  }

  return (
    <EntryCard title={m.account_signed_in()} leaving={leaving}>
      <div className="mt-7 flex flex-col gap-2">
        {session.data ? (
          <>
            <Tile label={m.entry_email()}>{session.data.user.email}</Tile>
            <Tile label={m.account_session_expires()}>
              {dateTime(session.data.session.expiresAt)}
            </Tile>
          </>
        ) : session.isError ? (
          <p className="py-2 text-row text-destructive">
            {failureText(session.error) ?? m.failure_unknown()}
          </p>
        ) : (
          <>
            <Skeleton className="h-16 w-full rounded-tile" />
            <Skeleton className="h-16 w-full rounded-tile" />
          </>
        )}
      </div>
      <p className="mt-5 text-row text-muted-foreground">
        {m.account_no_networks()}
      </p>
      <div className="mt-7 flex flex-col gap-2">
        {session.isError ? (
          <Button className={BUTTON} onClick={() => void session.refetch()}>
            {m.retry()}
          </Button>
        ) : null}
        <Button
          variant="outline"
          className={BUTTON}
          disabled={leaving}
          onClick={leave}
        >
          {m.sign_out()}
        </Button>
      </div>
    </EntryCard>
  )
}
