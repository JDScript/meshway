import {
  revokeAccountInvitation,
  type AccountInvitation,
  type AccountMember,
  type AccountRole,
} from "@meshway/control-client"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { UserPlusIcon, XIcon } from "lucide-react"
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
import { surfaceTitleId } from "@/features/shell/page-frame"
import { authed } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { unixDateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { InviteDialog } from "./invite-dialog"
import { invitationsQuery } from "./queries"
import {
  invitationState,
  invitationTime,
  type InvitationState,
} from "./invitation"
import { byRole, grantableRoles, roleLabel } from "./roles"
import type { MembersView } from "./url"

type Read<T> = { data: T | null; loading: boolean; failed: boolean }

/**
 * The members page's index: the network's members, or (for those who
 * manage it) its invitations. Choosing a member opens their workspace.
 */
export function MemberList({
  networkId,
  viewer,
  viewerId,
  view,
  members,
  invitations,
  openId,
  now,
  onView,
  onOpen,
}: {
  networkId: string
  /** This sign-in's role here; null while unread. */
  viewer: AccountRole | null
  viewerId: string | null
  view: MembersView
  members: Read<readonly AccountMember[]>
  invitations: Read<readonly AccountInvitation[]>
  openId: string | null
  /** Unix seconds. */
  now: number
  onView: (view: MembersView) => void
  onOpen: (userId: string) => void
}) {
  const [inviting, setInviting] = useState(false)
  const grantable = grantableRoles(viewer)
  const manages = grantable.length > 0
  const shown: MembersView = manages ? view : "members"
  const memberCount = members.data?.length ?? null

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PanelHeader
        page="members"
        title={m.members_title()}
        count={memberCount}
        action={
          manages ? (
            <PanelAction
              label={m.members_invite()}
              icon={UserPlusIcon}
              onClick={() => setInviting(true)}
            />
          ) : null
        }
      >
        {manages ? (
          <SegmentedControl<MembersView>
            label={m.members_view()}
            value={shown}
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
                value: "invitations",
                label: (
                  <SegmentCount
                    label={m.members_view_invitations()}
                    count={invitations.data?.length ?? null}
                  />
                ),
              },
            ]}
            onChange={onView}
            className="self-start"
          />
        ) : null}
      </PanelHeader>

      {shown === "members" ? (
        <Members
          members={members}
          viewerId={viewerId}
          openId={openId}
          onOpen={onOpen}
        />
      ) : (
        <Invitations
          networkId={networkId}
          invitations={invitations}
          members={members.data}
          now={now}
        />
      )}

      {manages ? (
        <InviteDialog
          networkId={networkId}
          roles={grantable}
          open={inviting}
          onOpenChange={setInviting}
        />
      ) : null}
    </div>
  )
}

function Members({
  members,
  viewerId,
  openId,
  onOpen,
}: {
  members: Read<readonly AccountMember[]>
  viewerId: string | null
  openId: string | null
  onOpen: (userId: string) => void
}) {
  const rows = useMemo(
    () =>
      [...(members.data ?? [])].sort(
        (a, b) =>
          byRole(a.membership.role, b.membership.role) ||
          a.user.email.localeCompare(b.user.email)
      ),
    [members.data]
  )

  const { ref, onKeyDown } = useListKeys({
    ids: rows.map((row) => row.user.user_id),
    openId,
    onOpen,
  })

  return (
    <PanelList
      ref={ref}
      aria-labelledby={surfaceTitleId("members", "panel")}
      onKeyDown={onKeyDown}
    >
      {members.loading ? <ListPlaceholders /> : null}
      {!members.loading && rows.length === 0 ? (
        <ListNote>
          {members.failed ? m.members_failed() : m.members_empty()}
        </ListNote>
      ) : null}
      {rows.map(({ user, membership }) => (
        <ListRow
          key={user.user_id}
          rowId={user.user_id}
          current={user.user_id === openId}
          onClick={() => onOpen(user.user_id)}
          lead={<InitialAvatar email={user.email} />}
          title={
            <>
              <span className="truncate">{user.email}</span>
              {user.user_id === viewerId ? <YouMark /> : null}
            </>
          }
          subtitle={
            <span className="truncate">{roleLabel[membership.role]()}</span>
          }
        />
      ))}
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

const stateLabel: Record<InvitationState, (at: string) => string> = {
  pending: (at) => m.members_invitation_expires({ at }),
  accepted: (at) => m.members_invitation_accepted({ at }),
  revoked: (at) => m.members_invitation_revoked({ at }),
  expired: (at) => m.members_invitation_expired({ at }),
}

function Invitations({
  networkId,
  invitations,
  members,
  now,
}: {
  networkId: string
  invitations: Read<readonly AccountInvitation[]>
  members: readonly AccountMember[] | null
  now: number
}) {
  const queryClient = useQueryClient()
  const [revoking, setRevoking] = useState<AccountInvitation | null>(null)
  const [confirming, setConfirming] = useState(false)
  const revoke = useMutation({
    mutationFn: (invitationId: string) =>
      authed((base, token) =>
        revokeAccountInvitation(base, token, networkId, {
          invitation_id: invitationId,
        })
      ),
    onSuccess: () => {
      setConfirming(false)
      toast.success(m.members_invitation_revoke_done())
    },
    onError: toastFailure,
    onSettled: () =>
      queryClient.invalidateQueries({
        queryKey: invitationsQuery(networkId).queryKey,
      }),
  })

  const rows = useMemo(
    () =>
      (invitations.data ?? [])
        .map((invitation) => {
          const state = invitationState(invitation, now)
          return { invitation, state, at: invitationTime(invitation, state) }
        })
        .sort((a, b) =>
          a.state === "pending" || b.state === "pending"
            ? Number(b.state === "pending") - Number(a.state === "pending") ||
              a.at - b.at
            : b.at - a.at
        ),
    [invitations.data, now]
  )
  const emailOf = (userId: string | null) =>
    members?.find((member) => member.user.user_id === userId)?.user.email

  return (
    <>
      <PanelList aria-labelledby={surfaceTitleId("members", "panel")}>
        {invitations.loading ? <ListPlaceholders /> : null}
        {!invitations.loading && rows.length === 0 ? (
          <ListNote>
            {invitations.failed
              ? m.members_invitations_failed()
              : m.members_invitations_empty()}
          </ListNote>
        ) : null}
        {rows.map(({ invitation, state, at }) => {
          const accepter =
            state === "accepted" ? emailOf(invitation.consumed_by) : undefined
          return (
            <StaticRow
              key={invitation.invitation_id}
              lead={<InitialAvatar email={invitation.email} />}
              title={
                <span
                  className={
                    state === "pending"
                      ? "truncate"
                      : "truncate text-muted-foreground"
                  }
                >
                  {invitation.email}
                </span>
              }
              subtitle={
                <span className="truncate">
                  {roleLabel[invitation.role]()} ·{" "}
                  {stateLabel[state](unixDateTime(at) ?? "—")}
                  {accepter && accepter !== invitation.email
                    ? ` · ${accepter}`
                    : ""}
                </span>
              }
              trail={
                <span className="flex size-7 items-center justify-center">
                  {state === "pending" ? (
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="rounded-full"
                      aria-label={`${m.members_invitation_revoke()} ${invitation.email}`}
                      onClick={() => {
                        setRevoking(invitation)
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
      </PanelList>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={m.members_invitation_revoke()}
        confirmLabel={m.members_invitation_revoke()}
        pending={revoke.isPending}
        onConfirm={() => {
          if (revoking) revoke.mutate(revoking.invitation_id)
        }}
        effects={[
          m.members_invitation_revoke_body({ email: revoking?.email ?? "" }),
        ]}
      />
    </>
  )
}
