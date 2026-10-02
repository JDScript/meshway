import {
  revokeAccountInvitation,
  type AccountInvitation,
  type AccountMember,
  type AccountRole,
} from "@meshway/control-client"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { ChevronRightIcon, UserPlusIcon, XIcon } from "lucide-react"
import { useMemo, useRef, useState, type KeyboardEvent } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
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

const PLACEHOLDER_ROWS = 4

type Read<T> = { data: T | null; loading: boolean; failed: boolean }

/** The initial of an email, the account's mark. */
export function Initial({ email }: { email: string }) {
  return (
    <span
      aria-hidden
      className="flex size-8 shrink-0 items-center justify-center rounded-full bg-fill-raised text-xs font-semibold uppercase"
    >
      {email[0] ?? "·"}
    </span>
  )
}

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
  const count =
    shown === "members"
      ? (members.data?.length ?? null)
      : (invitations.data?.length ?? null)

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-col gap-3 px-4 pt-4 pb-3">
        <div className="flex h-8 items-center gap-2">
          <h2
            id={surfaceTitleId("members", "panel")}
            className="text-base font-semibold tracking-tight"
          >
            {m.members_title()}
          </h2>
          <span className="flex-1 text-xs text-muted-foreground tabular-nums">
            {count ?? "—"}
          </span>
          {manages ? (
            <Button
              variant="secondary"
              size="sm"
              className="h-8 rounded-full pr-3.5 pl-3"
              onClick={() => setInviting(true)}
            >
              <UserPlusIcon />
              {m.members_invite()}
            </Button>
          ) : null}
        </div>
        {manages ? (
          <SegmentedControl<MembersView>
            label={m.members_view()}
            value={shown}
            options={[
              { value: "members", label: m.members_view_members() },
              { value: "invitations", label: m.members_view_invitations() },
            ]}
            onChange={onView}
            className="self-start"
          />
        ) : null}
      </header>

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

function Placeholders() {
  return Array.from({ length: PLACEHOLDER_ROWS }, (_, index) => (
    <li key={index} className="flex h-14 items-center gap-3 px-2">
      <Skeleton className="size-8 rounded-full" />
      <Skeleton className="h-3 w-36 rounded-full" />
    </li>
  ))
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
  const list = useRef<HTMLUListElement>(null)
  const rows = useMemo(
    () =>
      [...(members.data ?? [])].sort(
        (a, b) =>
          byRole(a.membership.role, b.membership.role) ||
          a.user.email.localeCompare(b.user.email)
      ),
    [members.data]
  )

  const onKeyDown = (event: KeyboardEvent<HTMLUListElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return
    event.preventDefault()
    const focused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement.dataset.userId
        : undefined
    const index = rows.findIndex(
      (row) => row.user.user_id === (focused ?? openId)
    )
    const next =
      rows[
        index === -1
          ? 0
          : Math.min(
              rows.length - 1,
              Math.max(0, index + (event.key === "ArrowDown" ? 1 : -1))
            )
      ]
    if (!next) return
    list.current
      ?.querySelector<HTMLButtonElement>(
        `[data-user-id="${next.user.user_id}"]`
      )
      ?.focus()
    if (openId) onOpen(next.user.user_id)
  }

  return (
    <ul
      ref={list}
      aria-labelledby={surfaceTitleId("members", "panel")}
      onKeyDown={onKeyDown}
      className="min-h-0 flex-1 overflow-y-auto px-2 pb-2"
    >
      {members.loading ? <Placeholders /> : null}
      {!members.loading && rows.length === 0 ? (
        <li className="flex h-14 items-center px-2 text-row text-muted-foreground">
          {members.failed ? m.members_failed() : m.members_empty()}
        </li>
      ) : null}
      {rows.map(({ user, membership }) => (
        <li key={user.user_id}>
          <button
            type="button"
            data-user-id={user.user_id}
            aria-current={user.user_id === openId ? "true" : undefined}
            onClick={() => onOpen(user.user_id)}
            className="list-row h-14 gap-3 px-2"
          >
            <Initial email={user.email} />
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5">
                <span className="truncate text-row font-medium">
                  {user.email}
                </span>
                {user.user_id === viewerId ? <YouMark /> : null}
              </span>
              <span className="block truncate text-meta text-muted-foreground">
                {roleLabel[membership.role]()}
              </span>
            </span>
            <ChevronRightIcon className="size-4 shrink-0 text-muted-foreground" />
          </button>
        </li>
      ))}
    </ul>
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
      <ul
        aria-labelledby={surfaceTitleId("members", "panel")}
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-2"
      >
        {invitations.loading ? <Placeholders /> : null}
        {!invitations.loading && rows.length === 0 ? (
          <li className="flex h-14 items-center px-2 text-row text-muted-foreground">
            {invitations.failed
              ? m.members_invitations_failed()
              : m.members_invitations_empty()}
          </li>
        ) : null}
        {rows.map(({ invitation, state, at }) => {
          const accepter =
            state === "accepted" ? emailOf(invitation.consumed_by) : undefined
          return (
            <li
              key={invitation.invitation_id}
              data-state={state}
              className="flex h-14 items-center gap-3 rounded-row px-2"
            >
              <Initial email={invitation.email} />
              <span className="min-w-0 flex-1">
                <span
                  className={
                    state === "pending"
                      ? "block truncate text-row font-medium"
                      : "block truncate text-row font-medium text-muted-foreground"
                  }
                >
                  {invitation.email}
                </span>
                <span className="block truncate text-meta text-muted-foreground">
                  {roleLabel[invitation.role]()} ·{" "}
                  {stateLabel[state](unixDateTime(at) ?? "—")}
                  {accepter && accepter !== invitation.email
                    ? ` · ${accepter}`
                    : ""}
                </span>
              </span>
              <span className="flex size-7 shrink-0 items-center justify-center">
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
            </li>
          )
        })}
      </ul>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={m.members_invitation_revoke()}
        confirmLabel={m.members_invitation_revoke()}
        pending={revoke.isPending}
        onConfirm={() => {
          if (revoking) revoke.mutate(revoking.invitation_id)
        }}
      >
        {m.members_invitation_revoke_body({ email: revoking?.email ?? "" })}
      </ConfirmDialog>
    </>
  )
}
