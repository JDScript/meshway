import type { ActivityAction, ActivityEvent } from "@meshway/control-client"
import { useInfiniteQuery } from "@tanstack/react-query"
import { HistoryIcon } from "lucide-react"

import { RefList, RefRow } from "@/components/chrome/ref-row"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { failureText } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { currentSession } from "@/lib/session"
import { m } from "@/paraglide/messages.js"

import { activityTarget, pagedCount, type ActivityTargetView } from "./model"
import { accountActivityQuery } from "./queries"
import { useLoadMore } from "./use-load-more"

const actionLabel: Record<ActivityAction, () => string> = {
  "account.register": () => m.account_action_register(),
  "session.login": () => m.account_action_login(),
  "session.logout": () => m.account_action_logout(),
  "session.revoke": () => m.account_action_revoke(),
  "session.revokeOthers": () => m.account_action_revoke_others(),
  "session.revokeAll": () => m.account_action_revoke_all(),
  "account.password": () => m.account_action_password(),
  "account.passwordReset": () => m.account_action_password_reset(),
  "account.resetIssued": () => m.account_action_reset_issued(),
  "account.email": () => m.account_action_email(),
  "network.create": () => m.account_action_network_create(),
  "network.rename": () => m.account_action_network_rename(),
  "configuration.apply": () => m.account_action_configuration_apply(),
  "membership.role": () => m.account_action_membership_role(),
  "membership.join": () => m.account_action_membership_join(),
  "membership.remove": () => m.account_action_membership_remove(),
  "joinLink.create": () => m.account_action_join_link_create(),
  "joinLink.revoke": () => m.account_action_join_link_revoke(),
}

function targetLabel(target: ActivityTargetView): string | null {
  switch (target.kind) {
    case "account":
      return null
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
  }
}

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
          <Skeleton className="h-12 w-full rounded-tile" />
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
      title={actionLabel[event.action]()}
      subtitle={
        event.actor.type === "system"
          ? m.account_activity_by_system({ at })
          : at
      }
      state={targetLabel(activityTarget(event.target, thisSession))}
    />
  )
}
