import {
  listNetworkActivity,
  type ActivityAction,
  type ActivityEvent,
  type NetworkSummary,
} from "@meshway/control-client"
import {
  infiniteQueryOptions,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query"

import { EmptyLine } from "@/components/chrome/section"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { actorId, actorText, type ActorWords } from "@/features/account/actor"
import { sessionQuery } from "@/features/auth/queries"
import { membersQuery } from "@/features/members/queries"
import { networkKey } from "@/features/network/queries"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { ApiFailure, authed, client, problemCode } from "@/lib/api"
import { failureText, toastFailure } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

/** Events per page; the controller takes 1 to 200. */
const ACTIVITY_PAGE = 50

/** What happened in the network, latest first; only those who manage members may read it. */
export const activityQuery = (networkId: string) =>
  infiniteQueryOptions({
    queryKey: [...networkKey(networkId), "activity"],
    queryFn: ({ pageParam, signal }) =>
      authed((token) =>
        listNetworkActivity(
          client,
          { networkId },
          pageParam === null
            ? { limit: ACTIVITY_PAGE }
            : { limit: ACTIVITY_PAGE, cursor: pageParam },
          { token, signal }
        )
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: 30_000,
  })

const actionLabel: Record<ActivityAction, () => string> = {
  "account.register": () => m.activity_account_register(),
  "session.login": () => m.activity_session_login(),
  "session.logout": () => m.activity_session_logout(),
  "session.revoke": () => m.activity_session_revoke(),
  "session.revokeOthers": () => m.activity_session_revoke_others(),
  "session.revokeAll": () => m.activity_session_revoke_all(),
  "account.password": () => m.activity_account_password(),
  "account.passwordReset": () => m.activity_account_password_reset(),
  "account.resetIssued": () => m.activity_account_reset_issued(),
  "account.email": () => m.activity_account_email(),
  "network.create": () => m.activity_network_create(),
  "network.rename": () => m.activity_network_rename(),
  "configuration.apply": () => m.activity_configuration_apply(),
  "membership.role": () => m.activity_membership_role(),
  "membership.join": () => m.activity_membership_join(),
  "membership.remove": () => m.activity_membership_remove(),
  "joinLink.create": () => m.activity_join_link_create(),
  "joinLink.revoke": () => m.activity_join_link_revoke(),
}

type Names = {
  byUser: ReadonlyMap<string, string>
  byMembership: ReadonlyMap<string, string>
  viewerId: string | null
}

/** Who did it, by the email they had then (else their ID), or the system. */
const actorWords: ActorWords = {
  system: () => m.activity_actor_system(),
  user: (id) => m.activity_actor_user({ id }),
  self: (name) => m.activity_actor_self({ name }),
}

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
  const activity = useInfiniteQuery(activityQuery(network.id))
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

  const loadMore = () =>
    void activity.fetchNextPage().then((result) => {
      if (!result.isFetchNextPageError) return
      if (
        result.error instanceof ApiFailure &&
        problemCode(result.error.failure) === "invalid_cursor"
      )
        void activity.refetch()
      else toastFailure(result.error)
    })

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
                    {actionLabel[event.action]()}
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
                    {actorText(event.actor, names.viewerId, actorWords)}
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
