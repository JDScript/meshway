import type { ActivityEvent } from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useInfiniteQuery } from "@tanstack/react-query"
import { HistoryIcon } from "lucide-react"

import { failureText } from "@/api/failure"
import { accountActivityQuery } from "@/api/queries/account"
import { RefList, RefRow } from "@/components/chrome/ref-row"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Skeleton } from "@/components/ui/skeleton"
import { useLoadMore } from "@/hooks/use-load-more"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import {
  activityTarget,
  pagedCount,
  type ActivityTargetView,
} from "@/lib/account"
import {
  activityActionLabel,
  ACTOR_WORDS,
  actorId,
  actorText,
  type ActorWords,
} from "@/lib/activity"
import { dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"
import { currentSession } from "@/state/session"

function targetLabel(target: ActivityTargetView): string | null {
  switch (target.kind) {
    case "account":
      return target.previousEmail
        ? m.account_target_previous_email({ email: target.previousEmail })
        : null
    case "this_session":
      return m.account_session_this()
    case "session":
      return m.account_target_session({ id: target.short })
    case "network":
      return m.account_target_network({ id: target.short })
    case "membership":
      return m.account_target_membership({ id: target.short })
    case "join_link":
      return m.account_target_join_link({ id: target.short })
    case "configuration":
      return m.revision({ revision: String(target.revision) })
    case "routing_selection":
      return target.profile === null
        ? m.activity_target_routing_selection_cleared()
        : m.activity_target_routing_selection({ profile: target.profile })
  }
}

// Every event here is the account's own, so the actor is not marked as this
// account; its email is the one recorded then, which may not be today's.
/** Every event in this list is this account's: its own name needs no mark. */
const actorWords: ActorWords = { ...ACTOR_WORDS, self: (name) => name }

/** What happened to the account, newest first, as the controller recorded it. */
export function ActivityList() {
  const activity = useInfiniteQuery(accountActivityQuery())
  const loadMore = useLoadMore(activity)
  useReadFailureToast(
    "account-activity",
    activity.isError,
    failureText(activity.error) ?? m.account_activity_failed()
  )
  const rows = activity.data?.pages.flatMap((page) => page.items) ?? []
  const unread = activity.data === undefined
  const thisSession = currentSession()?.id ?? null

  return (
    <Section
      title={m.account_activity()}
      count={unread ? null : pagedCount(rows.length, activity.hasNextPage)}
    >
      {unread ? (
        activity.isError ? (
          <EmptyLine>{m.account_activity_failed()}</EmptyLine>
        ) : (
          <div className="flex flex-col gap-1">
            {[0, 1, 2].map((row) => (
              <Skeleton key={row} className="h-12 w-full rounded-tile" />
            ))}
          </div>
        )
      ) : rows.length === 0 ? (
        <EmptyLine>{m.account_activity_empty()}</EmptyLine>
      ) : (
        <RefList>
          {rows.map((event) => (
            <ActivityRow
              key={event.id}
              event={event}
              thisSession={thisSession}
            />
          ))}
          {activity.hasNextPage ? (
            <li>
              <Button
                variant="ghost"
                className="h-10 w-full justify-start rounded-tile px-3.5 text-muted-foreground"
                disabled={activity.isFetchingNextPage}
                aria-busy={activity.isFetchingNextPage}
                onClick={loadMore}
              >
                {m.account_activity_more()}
              </Button>
            </li>
          ) : null}
        </RefList>
      )}
    </Section>
  )
}

function ActivityRow({
  event,
  thisSession,
}: {
  event: ActivityEvent
  thisSession: string | null
}) {
  const at = dateTime(event.at) ?? m.account_unknown()
  return (
    <RefRow
      lead={<HistoryIcon className="size-4 text-muted-foreground" />}
      title={activityActionLabel[event.action]()}
      subtitle={
        <span title={actorId(event.actor)}>
          {m.account_activity_by({
            at,
            actor: actorText(event.actor, null, actorWords),
          })}
        </span>
      }
      state={targetLabel(activityTarget(event.target, thisSession))}
    />
  )
}
