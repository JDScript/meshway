import type { ActivityEvent, NetworkSummary } from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useInfiniteQuery, useQuery } from "@tanstack/react-query"

import { failureText } from "@/api/failure"
import { sessionQuery } from "@/api/queries/auth"
import { membersQuery } from "@/api/queries/members"
import { networkActivityQuery } from "@/api/queries/network"
import { EmptyLine } from "@/components/chrome/section"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { useLoadMore } from "@/hooks/use-load-more"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import {
  activityActionLabel,
  ACTOR_WORDS,
  actorId,
  actorText,
} from "@/lib/activity"
import { dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

type Names = {
  byUser: ReadonlyMap<string, string>
  byMembership: ReadonlyMap<string, string>
  viewerId: string | null
}

/** Who did it, by the email they had then (else their ID), or the system. */
/** What it was done to, when that is more than the network itself. */
function targetName(event: ActivityEvent, names: Names): string | null {
  const { target } = event
  switch (target.type) {
    case "network":
      return null
    case "membership":
      return (
        names.byMembership.get(target.membershipId) ??
        m.activity_target_member()
      )
    case "account":
      return names.byUser.get(target.userId) ?? m.activity_target_account()
    case "joinLink":
      return m.activity_target_join_link()
    case "session":
      return m.activity_target_session()
    case "configuration":
      return m.revision({ revision: String(target.revision) })
    case "routingSelection":
      return target.profile === null
        ? m.activity_target_routing_selection_cleared()
        : m.activity_target_routing_selection({ profile: target.profile })
  }
}

/** The configuration revisions an event moved between, when it did. */
function revisions(event: ActivityEvent): string | null {
  const { beforeRevision: before, afterRevision: after } = event
  if (before !== null && after !== null && before !== after)
    return m.activity_revisions({ from: String(before), to: String(after) })
  return null
}

/** The network's activity: what was done, by whom and when, a page at a time. */
export function ActivitySection({ network }: { network: NetworkSummary }) {
  const activity = useInfiniteQuery(networkActivityQuery(network.id))
  const members = useInfiniteQuery(membersQuery(network.id))
  const session = useQuery(sessionQuery())

  useReadFailureToast(
    "activity",
    activity.isError && !activity.isFetchNextPageError,
    failureText(activity.error) ?? m.activity_failed()
  )

  const rows = activity.data?.pages.flatMap((page) => page.items) ?? []
  const listed = members.data?.pages.flatMap((page) => page.items) ?? []
  const names: Names = {
    byUser: new Map(listed.map((member) => [member.userId, member.email])),
    byMembership: new Map(
      listed.map((member) => [member.membershipId, member.email])
    ),
    viewerId: session.data?.user.id ?? null,
  }

  const loadMore = useLoadMore(activity)

  return (
    <div className="flex flex-col gap-2">
      {activity.data === undefined ? (
        activity.isError ? (
          <EmptyLine>{m.activity_failed()}</EmptyLine>
        ) : (
          <div className="flex flex-col gap-1">
            {[0, 1, 2].map((index) => (
              <Skeleton key={index} className="h-14 w-full rounded-tile" />
            ))}
          </div>
        )
      ) : rows.length === 0 ? (
        <EmptyLine>{m.activity_empty()}</EmptyLine>
      ) : (
        <ol className="flex flex-col gap-1">
          {rows.map((event) => {
            const target = targetName(event, names)
            const moved = revisions(event)
            return (
              <li
                key={event.id}
                className="flex min-h-14 items-center gap-3 rounded-tile bg-fill-subtle py-2 pr-3.5 pl-3.5"
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate text-row font-medium">
                    {activityActionLabel[event.action]()}
                    {target ? (
                      <span className="font-normal text-muted-foreground">
                        {" · "}
                        {target}
                      </span>
                    ) : null}
                  </div>
                  <div
                    className="truncate text-meta text-muted-foreground"
                    title={actorId(event.actor)}
                  >
                    {actorText(event.actor, names.viewerId, ACTOR_WORDS)}
                    {moved ? ` · ${moved}` : ""}
                  </div>
                </div>
                <time
                  dateTime={event.at}
                  className="shrink-0 text-meta text-muted-foreground tabular-nums"
                >
                  {dateTime(event.at) ?? "—"}
                </time>
              </li>
            )
          })}
        </ol>
      )}
      {activity.hasNextPage ? (
        <Button
          variant="ghost"
          className="h-10 self-start rounded-full px-4"
          disabled={activity.isFetchingNextPage}
          aria-busy={activity.isFetchingNextPage}
          onClick={loadMore}
        >
          {activity.isFetchingNextPage ? <Spinner /> : null}
          {m.activity_more()}
        </Button>
      ) : null}
    </div>
  )
}
