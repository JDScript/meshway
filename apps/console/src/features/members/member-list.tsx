import {
  revokeJoinLink,
  type JoinLinkListResponse,
  type JoinLinkView,
  type MemberSummary,
  type NetworkMembersResponse,
} from "@meshway/control-client"
import {
  useMutation,
  useQueryClient,
  type InfiniteData,
  type UseInfiniteQueryResult,
} from "@tanstack/react-query"
import { LinkIcon, XIcon } from "lucide-react"
import { useMemo, useState } from "react"
import { toast } from "sonner"

import { InitialAvatar } from "@/components/chrome/avatar"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import {
  ListNote,
  ListPlaceholders,
  ListRow,
  PanelList,
  StaticRow,
  useListKeys,
} from "@/components/chrome/list"
import { PanelAction, PanelHeader } from "@/components/chrome/panel-header"
import {
  SegmentCount,
  SegmentedControl,
} from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { roleLabel } from "@/features/network/roles"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { ApiFailure, authed, client, problemCode } from "@/lib/api"
import { toastFailure, toastFailureWith } from "@/lib/failure"
import { calendarDate, dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { byJoinLinkState, joinLinkState, type JoinLinkState } from "./join-link"
import { JoinLinkDialog } from "./join-link-dialog"
import { joinLinksQuery } from "./queries"
import type { MembersView } from "./url"

type Paged<T> = UseInfiniteQueryResult<InfiniteData<T>>
type MembersRead = Paged<NetworkMembersResponse>
type LinksRead = Paged<JoinLinkListResponse>

/** The rows of every page read so far. */
const rowsOf = <T,>(read: Paged<{ items: T[] }>): T[] =>
  read.data?.pages.flatMap((page) => page.items) ?? []

/** How many: the controller's total when it gives one, else the rows read ("100+" while more remain). */
function countOf(read: Paged<{ items: unknown[]; total?: number }>) {
  const pages = read.data?.pages
  if (!pages) return null
  const total = pages.at(-1)?.total
  if (total !== undefined) return total
  const rows = pages.reduce((sum, page) => sum + page.items.length, 0)
  return read.hasNextPage ? `${rows}+` : rows
}

/** "Load more" for a paged list; an expired cursor reads the list again from its start. */
function loadMore(read: Paged<unknown>) {
  void read.fetchNextPage().then((result) => {
    if (!result.isFetchNextPageError) return
    if (
      result.error instanceof ApiFailure &&
      problemCode(result.error.failure) === "invalid_cursor"
    )
      void read.refetch()
    else toastFailure(result.error)
  })
}

/**
 * The members page's index: the network's members, or its join links.
 * Choosing a member opens their workspace; a link is revoked in place.
 */
export function MemberList({
  networkId,
  view,
  members,
  links,
  viewerId,
  openId,
  now,
  onView,
  onOpen,
}: {
  networkId: string
  view: MembersView
  members: MembersRead
  links: LinksRead
  viewerId: string | null
  openId: string | null
  now: number
  onView: (view: MembersView) => void
  onOpen: (membershipId: string) => void
}) {
  const [issuing, setIssuing] = useState(false)
  const memberCount = countOf(members)
  const linkCount = countOf(links)

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PanelHeader
        page="members"
        title={m.members_title()}
        count={memberCount}
        action={
          <PanelAction
            label={m.members_link_create()}
            icon={LinkIcon}
            onClick={() => setIssuing(true)}
          />
        }
      >
        <SegmentedControl<MembersView>
          label={m.members_view()}
          value={view}
          options={[
            {
              value: "members",
              label: (
                <SegmentCount
                  label={m.members_view_members()}
                  count={typeof memberCount === "number" ? memberCount : null}
                />
              ),
            },
            {
              value: "links",
              label: (
                <SegmentCount
                  label={m.members_view_links()}
                  count={typeof linkCount === "number" ? linkCount : null}
                />
              ),
            },
          ]}
          onChange={onView}
          className="self-start"
        />
      </PanelHeader>

      {view === "members" ? (
        <Members
          members={members}
          viewerId={viewerId}
          openId={openId}
          onOpen={onOpen}
        />
      ) : (
        <Links
          networkId={networkId}
          links={links}
          members={rowsOf(members)}
          now={now}
        />
      )}

      <JoinLinkDialog
        networkId={networkId}
        open={issuing}
        onOpenChange={setIssuing}
      />
    </div>
  )
}

function MoreRow({ read }: { read: Paged<unknown> }) {
  if (!read.hasNextPage) return null
  return (
    <ListRow
      disabled={read.isFetchingNextPage}
      onClick={() => loadMore(read)}
      lead={read.isFetchingNextPage ? <Spinner className="size-3.5" /> : null}
      title={
        <span className="truncate text-muted-foreground">
          {m.members_more()}
        </span>
      }
    />
  )
}

function Members({
  members,
  viewerId,
  openId,
  onOpen,
}: {
  members: MembersRead
  viewerId: string | null
  openId: string | null
  onOpen: (membershipId: string) => void
}) {
  const rows = rowsOf(members)
  const unread = members.data === undefined
  const { ref, onKeyDown } = useListKeys({
    ids: rows.map((row) => row.membershipId),
    openId,
    onOpen,
  })

  return (
    <PanelList
      ref={ref}
      aria-labelledby={surfaceTitleId("members", "panel")}
      onKeyDown={onKeyDown}
    >
      {unread && !members.isError ? <ListPlaceholders /> : null}
      {unread && members.isError ? (
        <ListNote>{m.members_failed()}</ListNote>
      ) : null}
      {!unread && rows.length === 0 ? (
        <ListNote>{m.members_empty()}</ListNote>
      ) : null}
      {rows.map((member) => (
        <ListRow
          key={member.membershipId}
          rowId={member.membershipId}
          current={member.membershipId === openId}
          onClick={() => onOpen(member.membershipId)}
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
      <MoreRow read={members} />
    </PanelList>
  )
}

export function YouMark() {
  return (
    <span className="shrink-0 rounded-full bg-fill-raised px-1.5 text-meta font-medium text-muted-foreground">
      {m.members_you()}
    </span>
  )
}

const stateLabel: Record<JoinLinkState, (at: string) => string> = {
  pending: (at) => m.members_link_pending({ at }),
  used: (at) => m.members_link_used({ at }),
  revoked: (at) => m.members_link_revoked({ at }),
  expired: (at) => m.members_link_expired({ at }),
}

function Links({
  networkId,
  links,
  members,
  now,
}: {
  networkId: string
  links: LinksRead
  members: readonly MemberSummary[]
  now: number
}) {
  const queryClient = useQueryClient()
  const [revoking, setRevoking] = useState<JoinLinkView | null>(null)
  const [confirming, setConfirming] = useState(false)
  const revoke = useMutation({
    mutationFn: (linkId: string) =>
      authed((token) =>
        revokeJoinLink(client, { networkId, linkId }, { token })
      ),
    onSuccess: () => {
      setConfirming(false)
      toast.success(m.members_link_revoke_done())
    },
    onError: toastFailureWith({
      permission_denied: () => m.members_link_revoke_denied(),
    }),
    onSettled: () =>
      queryClient.invalidateQueries({
        queryKey: joinLinksQuery(networkId).queryKey,
      }),
  })

  const read = links.data
  const rows = useMemo(
    () =>
      (read?.pages.flatMap((page) => page.items) ?? [])
        .map((link) => ({ link, ...joinLinkState(link, now) }))
        .sort(byJoinLinkState),
    [read, now]
  )
  const emailOf = (userId: string | null) =>
    members.find((member) => member.userId === userId)?.email

  return (
    <>
      <PanelList aria-labelledby={surfaceTitleId("members", "panel")}>
        {read === undefined && !links.isError ? <ListPlaceholders /> : null}
        {read === undefined && links.isError ? (
          <ListNote>{m.members_links_failed()}</ListNote>
        ) : null}
        {read !== undefined && rows.length === 0 ? (
          <ListNote>{m.members_links_empty()}</ListNote>
        ) : null}
        {rows.map(({ link, state, at }) => {
          const user = state === "used" ? emailOf(link.consumedByUserId) : null
          return (
            <StaticRow
              key={link.id}
              lead={
                <LinkIcon
                  className={
                    state === "pending"
                      ? "size-4"
                      : "size-4 text-muted-foreground"
                  }
                />
              }
              title={
                <span
                  className={
                    state === "pending"
                      ? "truncate"
                      : "truncate text-muted-foreground"
                  }
                >
                  {m.members_link_title({ role: roleLabel[link.role]() })}
                </span>
              }
              subtitle={
                <span className="truncate">
                  {stateLabel[state](dateTime(at) ?? "—")}
                  {user ? ` · ${user}` : ""}
                </span>
              }
              trail={
                <span className="flex size-7 items-center justify-center">
                  {state === "pending" ? (
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="rounded-full"
                      aria-label={m.members_link_revoke()}
                      onClick={() => {
                        setRevoking(link)
                        setConfirming(true)
                      }}
                    >
                      <XIcon />
                    </Button>
                  ) : null}
                </span>
              }
            />
          )
        })}
        <MoreRow read={links} />
      </PanelList>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={m.members_link_revoke()}
        confirmLabel={m.members_link_revoke()}
        pending={revoke.isPending}
        onConfirm={() => {
          if (revoking) revoke.mutate(revoking.id)
        }}
        effects={[
          m.members_link_revoke_effect({
            role: revoking ? roleLabel[revoking.role]() : "",
            at: revoking ? (dateTime(revoking.expiresAt) ?? "—") : "",
          }),
        ]}
      />
    </>
  )
}
