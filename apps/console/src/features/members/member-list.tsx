import type { NetworkMembersResponse } from "@meshway/control-client"
import type {
  InfiniteData,
  UseInfiniteQueryResult,
} from "@tanstack/react-query"

import { InitialAvatar } from "@/components/chrome/avatar"
import {
  ListNote,
  ListPlaceholders,
  ListRow,
  PanelList,
  StaticRow,
} from "@/components/chrome/list"
import { PanelHeader } from "@/components/chrome/panel-header"
import { Spinner } from "@/components/ui/spinner"
import { roleLabel } from "@/features/network/roles"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { ApiFailure, problemCode } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { calendarDate } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

type MembersRead = UseInfiniteQueryResult<InfiniteData<NetworkMembersResponse>>

/**
 * The members page's index: the network's members as the controller lists
 * them, a page at a time. Read only: changing roles, removing members and
 * inviting are not offered by this controller.
 */
export function MemberList({
  members,
  viewerId,
}: {
  members: MembersRead
  viewerId: string | null
}) {
  const rows = members.data?.pages.flatMap((page) => page.items) ?? []
  const unread = members.data === undefined
  const count = unread
    ? null
    : members.hasNextPage
      ? `${rows.length}+`
      : rows.length

  const loadMore = () =>
    void members.fetchNextPage().then((result) => {
      if (!result.isFetchNextPageError) return
      // An expired cursor: read the list again from its start.
      if (
        result.error instanceof ApiFailure &&
        problemCode(result.error.failure) === "invalid_cursor"
      )
        void members.refetch()
      else toastFailure(result.error)
    })

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PanelHeader page="members" title={m.members_title()} count={count} />
      <PanelList aria-labelledby={surfaceTitleId("members", "panel")}>
        {unread && !members.isError ? <ListPlaceholders /> : null}
        {unread && members.isError ? (
          <ListNote>{m.members_failed()}</ListNote>
        ) : null}
        {!unread && rows.length === 0 ? (
          <ListNote>{m.members_empty()}</ListNote>
        ) : null}
        {rows.map((member) => (
          <StaticRow
            key={member.membershipId}
            lead={<InitialAvatar email={member.email} />}
            title={
              <>
                <span className="truncate">{member.email}</span>
                {member.userId === viewerId ? <YouMark /> : null}
              </>
            }
            subtitle={
              <span className="truncate">
                {m.members_joined({
                  role: roleLabel[member.role](),
                  date: calendarDate(member.joinedAt) ?? "—",
                })}
              </span>
            }
          />
        ))}
        {members.hasNextPage ? (
          <ListRow
            disabled={members.isFetchingNextPage}
            onClick={loadMore}
            lead={
              members.isFetchingNextPage ? (
                <Spinner className="size-3.5" />
              ) : null
            }
            title={
              <span className="truncate text-muted-foreground">
                {m.members_more()}
              </span>
            }
          />
        ) : null}
      </PanelList>
    </div>
  )
}

function YouMark() {
  return (
    <span className="shrink-0 rounded-full bg-fill-raised px-1.5 text-meta font-medium text-muted-foreground">
      {m.members_you()}
    </span>
  )
}
