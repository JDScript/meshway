"use client"

import type { AccountInvitation, AccountMember, AccountMembershipChange, NodeOwnership, NodeSummary } from "@meshway/control-client"
import { ChevronDownIcon, CircleAlertIcon, EllipsisIcon, LogOutIcon, ServerIcon, UserMinusIcon, UserPlusIcon } from "lucide-react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useState, useTransition, type ReactNode } from "react"

import { RefreshButton } from "@/components/common/refresh-button"
import { StatusBadge, type Tone } from "@/components/common/status-badge"
import { InvitationIssueForm } from "@/components/auth/invitation-issue-form"
import { useActionScope } from "@/components/network/network-workspace"
import { LastContact } from "@/components/nodes/node-status"
import { useRoutes } from "@/components/shell/network-context"
import { PageHeader } from "@/components/shell/page-header"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { roleLabels } from "@/lib/account-model"
import {
  canManageMembers,
  grantableRoles,
  invitationStatus,
  invitationStatusLabels,
  memberChanges,
  ownershipView,
  sortInvitations,
  sortMembers,
  type InvitationStatus,
  type MemberActor,
  type OwnershipView,
} from "@/lib/members-model"
import { claimNodeAction, leaveNetworkAction, revokeInvitationAction, saveMemberAction } from "@/lib/member-actions"
import { formatClock, formatEpochSeconds } from "@/lib/time"
import { cn } from "@/lib/utils"

type Read<T> = { ok: true } & T | { ok: false; error: string }

const roleTone: Record<AccountMember["membership"]["role"], Tone> = { owner: "info", admin: "neutral", member: "neutral" }
const invitationTone: Record<InvitationStatus, Tone> = { pending: "warning", accepted: "success", revoked: "neutral", expired: "neutral" }

export function MembersWorkspace({
  actor,
  identityError,
  members,
  fetchedAt,
  ownership,
  nodes,
  invitations,
  openInvite,
}: {
  actor: MemberActor
  identityError: string | null
  members: AccountMember[]
  fetchedAt: number
  ownership: Read<{ entries: NodeOwnership[] }>
  nodes: Read<{ list: NodeSummary[] }>
  /** null: this role cannot read invitations. */
  invitations: Read<{ list: AccountInvitation[] }> | null
  openInvite: boolean
}) {
  const router = useRouter()
  const routes = useRoutes()
  const [inviting, setInviting] = useState(openInvite)
  const manage = canManageMembers(actor)
  const roles = grantableRoles(actor)
  const sorted = sortMembers(members)
  const view: OwnershipView | null = ownership.ok && nodes.ok ? ownershipView(ownership.entries, members, nodes.list) : null
  const now = Math.floor(fetchedAt / 1000)
  const pendingInvitations = invitations?.ok ? invitations.list.filter((invitation) => invitationStatus(invitation, now) === "pending").length : 0
  const closeInvite = () => {
    setInviting(false)
    if (openInvite) router.replace(routes.members)
  }

  return (
    <>
      <PageHeader
        title="成员"
        description={
          actor.kind === "bootstrap"
            ? "以实例管理员令牌查看。日常管理请用所有者或管理员账户登录。"
            : actor.role === "member"
              ? "你是本网络的成员，可以查看成员与设备归属。"
              : undefined
        }
        meta={
          <>
            <span>{members.length} 位成员</span>
            {pendingInvitations > 0 ? <span>{pendingInvitations} 个待接受邀请</span> : null}
            <span>读取于 {formatClock(fetchedAt)}</span>
          </>
        }
        actions={
          <>
            <RefreshButton />
            {manage ? (
              <Button onClick={() => setInviting(true)}>
                <UserPlusIcon data-icon="inline-start" />
                邀请成员
              </Button>
            ) : null}
          </>
        }
      />
      {identityError ? (
        <p role="alert" className="text-xs text-destructive">无法确认你的角色，暂不提供修改：{identityError}</p>
      ) : null}

      <section aria-label="成员" className="overflow-hidden rounded-xl border bg-card shadow-xs">
        <ul className="divide-y">
          {sorted.map((member) => (
            <MemberRow
              key={member.membership.membership_id}
              actor={actor}
              member={member}
              devices={view?.byMembership.get(member.membership.membership_id) ?? null}
              devicesError={!ownership.ok ? ownership.error : !nodes.ok ? nodes.error : null}
              fetchedAt={fetchedAt}
            />
          ))}
        </ul>
        {sorted.length === 0 ? <p className="px-5 py-8 text-center text-[0.8125rem] text-muted-foreground">这个网络还没有成员。</p> : null}
      </section>

      {invitations ? <Invitations invitations={invitations} members={members} now={now} /> : null}

      {view && (view.unassigned.length > 0 || view.formerMember.length > 0) ? (
        <UnassignedDevices actor={actor} view={view} members={sorted} fetchedAt={fetchedAt} />
      ) : null}

      <Dialog open={inviting} onOpenChange={(open) => (open ? setInviting(true) : closeInvite())}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>邀请成员</DialogTitle>
            <DialogDescription>生成一次性邀请链接。受邀人用该邮箱注册，或用已有账户接受。不会发送邮件。</DialogDescription>
          </DialogHeader>
          {inviting ? <InvitationIssueForm roles={roles} onDone={closeInvite} /> : null}
        </DialogContent>
      </Dialog>
    </>
  )
}

function initials(email: string): string {
  return email.slice(0, 1).toUpperCase()
}

const changeCopy: Record<AccountMembershipChange, { menu: string; title: (email: string) => string; body: string; confirm: string }> = {
  owner: { menu: "设为所有者", title: (email) => `将 ${email} 设为所有者？`, body: "所有者可以管理所有成员与角色，以及全部网络配置。", confirm: "设为所有者" },
  admin: {
    menu: "设为管理员",
    title: (email) => `将 ${email} 设为管理员？`,
    body: "管理员可以修改网络配置、邀请和移除成员，但不能调整角色。从所有者降级时，其发出的未使用所有者 / 管理员邀请会失效。",
    confirm: "设为管理员",
  },
  member: {
    menu: "设为成员",
    title: (email) => `将 ${email} 设为成员？`,
    body: "成员只能查看本网络，并接入自己的设备。其发出的所有未使用邀请会失效。",
    confirm: "设为成员",
  },
  remove: {
    menu: "移除…",
    title: (email) => `从网络移除 ${email}？`,
    body: "该成员在本网络的设备会同时撤销。重新邀请后会成为新的成员，旧设备不会恢复。",
    confirm: "移除",
  },
}

function MemberRow({
  actor,
  member,
  devices,
  devicesError,
  fetchedAt,
}: {
  actor: MemberActor
  member: AccountMember
  devices: NodeSummary[] | null
  devicesError: string | null
  fetchedAt: number
}) {
  const routes = useRoutes()
  const scope = useActionScope()
  const [pending, start] = useTransition()
  const [change, setChange] = useState<AccountMembershipChange | null>(null)
  const [leaving, setLeaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [expanded, setExpanded] = useState(false)
  const self = actor.kind === "user" && actor.userId === member.user.user_id
  const changes = memberChanges(actor, member)
  const active = devices?.filter((node) => node.revoked_at === null) ?? []
  const revoked = (devices?.length ?? 0) - active.length

  const apply = (next: AccountMembershipChange) =>
    start(async () => {
      const result = await saveMemberAction(scope, member.user.user_id, next)
      setError(result.status === "error" ? result.message : null)
      setChange(null)
    })
  const leave = () =>
    start(async () => {
      const result = await leaveNetworkAction(scope)
      setError(result.status === "error" ? result.message : null)
      setLeaving(false)
    })

  return (
    <li className="flex flex-col">
      <div className="flex min-w-0 items-center gap-3 px-4 py-3 sm:px-5">
        <Avatar className="size-8 rounded-full">
          <AvatarFallback className={cn("text-xs font-semibold", self ? "bg-primary text-primary-foreground" : "bg-muted")}>{initials(member.user.email)}</AvatarFallback>
        </Avatar>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="flex min-w-0 items-center gap-2">
            <span className="truncate text-sm font-medium">{member.user.email}</span>
            {self ? <span className="shrink-0 text-xs text-muted-foreground">你</span> : null}
          </span>
          <button
            type="button"
            onClick={() => setExpanded((value) => !value)}
            aria-expanded={expanded}
            disabled={devices === null && devicesError === null}
            className="inline-flex w-fit items-center gap-1 rounded text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
          >
            <ServerIcon className="size-3" aria-hidden="true" />
            {devices === null ? (devicesError ? "设备归属读取失败" : "—") : active.length === 0 ? "没有设备" : `${active.length} 台设备`}
            {revoked > 0 ? <span>· {revoked} 台已撤销</span> : null}
            <ChevronDownIcon className={cn("size-3 transition-transform", expanded && "rotate-180")} aria-hidden="true" />
          </button>
        </div>
        <StatusBadge tone={roleTone[member.membership.role]} className="h-6 px-2.5 text-xs">
          {roleLabels[member.membership.role]}
        </StatusBadge>
        {changes.length > 0 || self ? (
          <DropdownMenu>
            <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label={`${member.user.email} 的操作`} disabled={pending} />}>
              <EllipsisIcon />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-44">
              {changes.some((item) => item !== "remove") ? (
                <DropdownMenuGroup>
                  <DropdownMenuLabel className="text-xs text-muted-foreground">角色</DropdownMenuLabel>
                  {changes
                    .filter((item) => item !== "remove")
                    .map((item) => (
                      <DropdownMenuItem key={item} onClick={() => setChange(item)}>
                        {changeCopy[item].menu}
                      </DropdownMenuItem>
                    ))}
                </DropdownMenuGroup>
              ) : null}
              {changes.includes("remove") ? (
                <>
                  {changes.length > 1 ? <DropdownMenuSeparator /> : null}
                  <DropdownMenuItem variant="destructive" onClick={() => setChange("remove")}>
                    <UserMinusIcon />
                    从网络移除…
                  </DropdownMenuItem>
                </>
              ) : null}
              {self ? (
                <>
                  {changes.length > 0 ? <DropdownMenuSeparator /> : null}
                  <DropdownMenuItem render={<Link href={routes.account} />}>我的账户</DropdownMenuItem>
                  <DropdownMenuItem variant="destructive" onClick={() => setLeaving(true)}>
                    <LogOutIcon />
                    离开网络…
                  </DropdownMenuItem>
                </>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
        ) : (
          <span className="size-7" aria-hidden="true" />
        )}
      </div>
      {error ? (
        <p role="alert" className="flex items-start gap-1.5 px-4 pb-3 text-xs text-destructive sm:px-5 sm:pl-16">
          <CircleAlertIcon className="mt-px size-3.5 shrink-0" />
          {error}
        </p>
      ) : null}
      {expanded ? (
        <div className="border-t bg-muted/25 px-4 py-2 sm:px-5 sm:pl-16">
          {devicesError ? (
            <p className="py-1 text-xs text-destructive">{devicesError}</p>
          ) : devices && devices.length > 0 ? (
            <DeviceList nodes={devices} fetchedAt={fetchedAt} />
          ) : (
            <p className="py-1 text-xs text-muted-foreground">这位成员还没有在本网络接入设备。设备由成员本人通过 Meshway App 登录接入。</p>
          )}
        </div>
      ) : null}

      <AlertDialog open={change !== null} onOpenChange={(open) => (open ? null : setChange(null))}>
        <AlertDialogContent>
          {change ? (
            <>
              <AlertDialogHeader>
                <AlertDialogTitle>{changeCopy[change].title(member.user.email)}</AlertDialogTitle>
                <AlertDialogDescription>
                  {changeCopy[change].body}
                  {change === "remove" && active.length > 0 ? ` 将撤销 ${active.length} 台设备。` : null}
                  {self ? " 这是你自己的角色，降级后你可能失去管理权限。" : null}
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel disabled={pending}>取消</AlertDialogCancel>
                <Button variant={change === "remove" ? "destructive" : "default"} disabled={pending} onClick={() => apply(change)}>
                  {pending ? "正在保存" : changeCopy[change].confirm}
                </Button>
              </AlertDialogFooter>
            </>
          ) : null}
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={leaving} onOpenChange={setLeaving}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>离开这个网络？</AlertDialogTitle>
            <AlertDialogDescription>
              你在本网络的成员身份与 {active.length} 台设备会同时撤销。之后需要新的邀请才能重新加入，旧设备不会恢复。你的账户和其他网络不受影响。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending}>取消</AlertDialogCancel>
            <Button variant="destructive" disabled={pending} onClick={leave}>
              {pending ? "正在离开" : "离开网络"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </li>
  )
}

function DeviceList({ nodes, fetchedAt }: { nodes: NodeSummary[]; fetchedAt: number }) {
  const routes = useRoutes()
  return (
    <ul className="flex flex-col">
      {nodes.map((node) => (
        <li key={node.id} className="flex min-w-0 items-center gap-3 py-1.5 text-[0.8125rem]">
          <Link href={routes.machine(node.id)} className={cn("min-w-0 truncate font-medium underline-offset-4 hover:underline", node.revoked_at !== null && "text-muted-foreground")}>
            {node.name}
          </Link>
          <span className="hidden font-mono text-xs text-muted-foreground sm:inline">{node.mesh_ipv4}</span>
          <span className="ml-auto text-xs text-muted-foreground">
            {node.revoked_at !== null ? <StatusBadge tone="danger">已撤销</StatusBadge> : <LastContact node={node} fetchedAt={fetchedAt} />}
          </span>
        </li>
      ))}
    </ul>
  )
}

function SectionHeading({ title, detail, children }: { title: string; detail?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-2">
      <div className="flex flex-col gap-0.5">
        <h2 className="text-[0.9375rem] font-semibold tracking-tight">{title}</h2>
        {detail ? <p className="text-xs text-muted-foreground">{detail}</p> : null}
      </div>
      {children}
    </div>
  )
}

function Invitations({ invitations, members, now }: { invitations: Read<{ list: AccountInvitation[] }>; members: AccountMember[]; now: number }) {
  const [showAll, setShowAll] = useState(false)
  if (!invitations.ok) {
    return (
      <section className="flex flex-col gap-3">
        <SectionHeading title="邀请" />
        <p role="alert" className="text-xs text-destructive">邀请读取失败：{invitations.error}</p>
      </section>
    )
  }
  const emails = new Map(members.map((member) => [member.user.user_id, member.user.email]))
  const sorted = sortInvitations(invitations.list, now)
  const pending = sorted.filter((invitation) => invitationStatus(invitation, now) === "pending")
  const shown = showAll ? sorted : pending
  const history = sorted.length - pending.length
  if (sorted.length === 0) return null
  return (
    <section aria-label="邀请" className="flex flex-col gap-3">
      <SectionHeading title="邀请" detail={pending.length === 0 ? "没有待接受的邀请。" : undefined}>
        {history > 0 ? (
          <Button variant="ghost" size="sm" onClick={() => setShowAll((value) => !value)} aria-pressed={showAll}>
            {showAll ? "只看待接受" : `显示全部（含 ${history} 条历史）`}
          </Button>
        ) : null}
      </SectionHeading>
      {shown.length > 0 ? (
        <ul className="divide-y overflow-hidden rounded-xl border bg-card shadow-xs">
          {shown.map((invitation) => (
            <InvitationRow
              key={invitation.invitation_id}
              invitation={invitation}
              status={invitationStatus(invitation, now)}
              issuer={invitation.issuer_user_id === null ? "实例管理员" : (emails.get(invitation.issuer_user_id) ?? "已离开的成员")}
              consumer={invitation.consumed_by === null ? null : (emails.get(invitation.consumed_by) ?? null)}
            />
          ))}
        </ul>
      ) : null}
    </section>
  )
}

function InvitationRow({ invitation, status, issuer, consumer }: { invitation: AccountInvitation; status: InvitationStatus; issuer: string; consumer: string | null }) {
  const scope = useActionScope()
  const [pending, start] = useTransition()
  const [error, setError] = useState<string | null>(null)
  const time =
    status === "pending"
      ? `${formatEpochSeconds(invitation.expires_at)} 到期`
      : status === "accepted" && invitation.consumed_at !== null
        ? `${formatEpochSeconds(invitation.consumed_at)} 接受${consumer && consumer !== invitation.email ? ` · ${consumer}` : ""}`
        : status === "revoked" && invitation.revoked_at !== null
          ? `${formatEpochSeconds(invitation.revoked_at)} 撤销`
          : `${formatEpochSeconds(invitation.expires_at)} 过期`
  return (
    <li className="flex flex-col gap-1 px-4 py-3 sm:px-5">
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
        <span className={cn("min-w-0 truncate text-sm", status === "pending" ? "font-medium" : "text-muted-foreground")}>{invitation.email}</span>
        <span className="text-xs text-muted-foreground">{roleLabels[invitation.role]}</span>
        <StatusBadge tone={invitationTone[status]}>{invitationStatusLabels[status]}</StatusBadge>
        <span className="text-xs text-muted-foreground tabular-nums">{time}</span>
        <span className="text-xs text-muted-foreground">由 {issuer} 发出</span>
        {status === "pending" ? (
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto"
            disabled={pending}
            onClick={() =>
              start(async () => {
                const result = await revokeInvitationAction(scope, invitation.invitation_id)
                setError(result.status === "error" ? result.message : null)
              })
            }
          >
            {pending ? "正在撤销" : "撤销"}
          </Button>
        ) : null}
      </div>
      {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
    </li>
  )
}

/**
 * Legacy nodes enrolled before accounts belong to nobody. An Owner assigns
 * each to a member explicitly; nothing is assigned automatically.
 */
function UnassignedDevices({ actor, view, members, fetchedAt }: { actor: MemberActor; view: OwnershipView; members: AccountMember[]; fetchedAt: number }) {
  const canClaim = actor.kind === "user" && actor.role === "owner"
  return (
    <section aria-label="未分配的设备" className="flex flex-col gap-3">
      <SectionHeading
        title="未分配的设备"
        detail={
          canClaim
            ? "账户上线前接入的节点还不属于任何成员。把它分配给实际使用者；节点身份、地址与标签不变。"
            : "账户上线前接入的节点还不属于任何成员，由所有者账户分配。"
        }
      />
      <ul className="divide-y overflow-hidden rounded-xl border bg-card shadow-xs">
        {view.unassigned.map((node) => (
          <UnassignedRow key={node.id} node={node} members={members} canClaim={canClaim} fetchedAt={fetchedAt} />
        ))}
        {view.formerMember.map((node) => (
          <li key={node.id} className="flex min-w-0 items-center gap-3 px-4 py-2.5 text-[0.8125rem] sm:px-5">
            <ServerIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 truncate">{node.name}</span>
            <span className="ml-auto text-xs text-muted-foreground">属于已离开的成员{node.revoked_at !== null ? " · 已撤销" : ""}</span>
          </li>
        ))}
      </ul>
    </section>
  )
}

function UnassignedRow({ node, members, canClaim, fetchedAt }: { node: NodeSummary; members: AccountMember[]; canClaim: boolean; fetchedAt: number }) {
  const routes = useRoutes()
  const scope = useActionScope()
  const [target, setTarget] = useState(members[0]?.membership.membership_id ?? "")
  const [pending, start] = useTransition()
  const [error, setError] = useState<string | null>(null)
  return (
    <li className="flex flex-col gap-1 px-4 py-2.5 sm:px-5">
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <ServerIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <Link href={routes.machine(node.id)} className="min-w-0 truncate text-[0.8125rem] font-medium underline-offset-4 hover:underline">
          {node.name}
        </Link>
        <span className="font-mono text-xs text-muted-foreground">{node.mesh_ipv4}</span>
        <span className="text-xs text-muted-foreground">
          <LastContact node={node} fetchedAt={fetchedAt} />
        </span>
        {canClaim ? (
          <span className="ml-auto flex items-center gap-2">
            <NativeSelect size="sm" value={target} onChange={(event) => setTarget(event.target.value)} disabled={pending} aria-label={`把 ${node.name} 分配给`}>
              {members.map((member) => (
                <NativeSelectOption key={member.membership.membership_id} value={member.membership.membership_id}>
                  {member.user.email}
                </NativeSelectOption>
              ))}
            </NativeSelect>
            <Button
              size="sm"
              variant="outline"
              disabled={pending || target === ""}
              onClick={() =>
                start(async () => {
                  const result = await claimNodeAction(scope, node.id, target)
                  setError(result.status === "error" ? result.message : null)
                })
              }
            >
              {pending ? "正在分配" : "分配"}
            </Button>
          </span>
        ) : null}
      </div>
      {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
    </li>
  )
}
