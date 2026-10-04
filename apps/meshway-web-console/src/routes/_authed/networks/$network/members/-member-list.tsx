import {
  revokeJoinLink,
  type JoinLinkListResponse,
  type JoinLinkView,
  type MembershipRole,
  type NetworkMembersResponse,
} from "@meshway/control-client"
import {
  useMutation,
  useQueryClient,
  type InfiniteData,
  type UseInfiniteQueryResult,
} from "@tanstack/react-query"
import { LinkIcon, XIcon } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { authed, client, codeOf } from "@/api/client"
import { toastFailureWith } from "@/api/failure"
import { joinLinksQuery } from "@/api/queries/members"
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
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { useLoadMore } from "@/hooks/use-load-more"
import { calendarDate, dateTime } from "@/lib/format"
import { joinLinkState, revocable, type JoinLinkState } from "@/lib/join-links"
import { roleLabel } from "@/lib/roles"
import { m } from "@/paraglide/messages.js"

import { JoinLinkDialog } from "./-join-link-dialog"
import type { MembersView } from "./-search"

type Paged<T> = UseInfiniteQueryResult<InfiniteData<T>>
type MembersRead = Paged<NetworkMembersResponse>
type LinksRead = Paged<JoinLinkListResponse>

/** The rows of every page read so far. */
const rowsOf = <T,>(read: Paged<{ items: T[] }>): T[] =>
  read.data?.pages.flatMap((page) => page.items) ?? []

/** How many there are: the controller's `total`, as of its latest page; never the rows read. */
const totalOf = (read: Paged<{ total: number }>) =>
  read.data?.pages.at(-1)?.total ?? null

/**
 * The members page's index: the network's members, or its join links
 * (newest first, as the controller pages them). Choosing a member opens
 * their workspace; a link is revoked in place where the controller allows.
 * Issuing is offered only with roles the controller lets this account issue.
 */
export function MemberList({
  networkId,
  joinLinkRoles,
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
  joinLinkRoles: readonly MembershipRole[]
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
  const memberCount = totalOf(members)
  const linkCount = totalOf(links)
  const mayIssue = joinLinkRoles.length > 0

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PanelHeader
        page="members"
        title={m.members_title()}
        count={memberCount}
        action={
          mayIssue ? (
            <PanelAction
              label={m.members_link_create()}
              icon={LinkIcon}
              onClick={() => setIssuing(true)}
            />
          ) : null
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
                  count={memberCount}
                />
              ),
            },
            {
              value: "links",
              label: (
                <SegmentCount
                  label={m.members_view_links()}
                  count={linkCount}
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
        <Links networkId={networkId} links={links} now={now} />
      )}

      <JoinLinkDialog
        networkId={networkId}
        roles={joinLinkRoles}
        open={issuing && mayIssue}
        onOpenChange={setIssuing}
      />
    </div>
  )
}

function MoreRow({ read }: { read: Paged<unknown> }) {
  const loadMore = useLoadMore(read)
  if (!read.hasNextPage) return null
  return (
    <ListRow
      disabled={read.isFetchingNextPage}
      onClick={loadMore}
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
  now,
}: {
  networkId: string
  links: LinksRead
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
    onError: (error) => {
      // Used, expired or revoked before: the list read again shows which.
      if (codeOf(error) === "state_conflict") {
        setConfirming(false)
        toast.warning(m.members_link_revoke_moot())
      } else
        toastFailureWith({
          permission_denied: () => m.members_link_revoke_denied(),
        })(error)
    },
    onSettled: () =>
      queryClient.invalidateQueries({
        queryKey: joinLinksQuery(networkId).queryKey,
      }),
  })

  const read = links.data
  const rows = rowsOf(links).map((link) => ({
    link,
    ...joinLinkState(link, now),
  }))
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
          // A used link says who joined with it; any other, who issued it.
          const who =
            state === "used" && link.consumedBy
              ? m.members_link_used_by({ email: link.consumedBy.email })
              : m.members_link_issued_by({ email: link.issuedBy.email })
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
                  {stateLabel[state](dateTime(at) ?? "—")} · {who}
                </span>
              }
              trail={
                <span className="flex size-7 items-center justify-center">
                  {revocable(link, state) ? (
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
