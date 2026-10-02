import {
  claimNodeOwnership,
  leaveAccountNetwork,
  saveAccountMember,
  type AccountMember,
  type AccountMembershipChange,
  type AccountRole,
  type ConnectionRuntimesSnapshot,
  type NodeOwnershipList,
  type NodeSummary,
} from "@meshway/control-client"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { ChevronRightIcon, LogOutIcon, UserMinusIcon } from "lucide-react"
import { useMemo, useState, type FormEvent, type ReactNode } from "react"
import { toast } from "sonner"

import { CloseButton } from "@/components/chrome/close-button"
import { FormSection } from "@/components/chrome/form"
import { ConfirmDialog, GlassDialog } from "@/components/chrome/glass-dialog"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import {
  networksQuery,
  nodesQuery,
  sessionQuery,
} from "@/features/network/queries"
import { useNetwork } from "@/features/network/use-network-view"
import { vitalityLabel } from "@/features/overview/labels"
import { vitalityOf } from "@/features/overview/model"
import { VitalityDot } from "@/features/overview/status-dots"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { authed } from "@/lib/api"
import { failureOf, toastFailure } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { YouMark } from "./member-list"
import { invitationsQuery, membersQuery, ownershipQuery } from "./queries"
import { canChangeRole, canRemove, ROLES, roleLabel } from "./roles"

type Read<T> = { data: T | null; loading: boolean }
type Runtimes = ConnectionRuntimesSnapshot | null

/** A refused change on the last Owner says so; anything else as usual. */
function toastMembershipFailure(error: unknown) {
  if (failureOf(error) === "conflict")
    toast.error(m.members_last_owner(), { id: "last-owner" })
  else toastFailure(error)
}

/**
 * One member in full: their role, their devices in this network, and
 * removal (or, for oneself, leaving). Owners also assign nodes no member
 * owns yet. Members see the same, without the controls.
 */
export function MemberWorkspace({
  networkId,
  userId,
  members,
  ownership,
  viewer,
  viewerId,
  onClose,
  onOpenNode,
}: {
  networkId: string
  userId: string
  members: Read<readonly AccountMember[]>
  ownership: Read<NodeOwnershipList>
  viewer: AccountRole | null
  viewerId: string | null
  onClose: () => void
  onOpenNode: (nodeId: string) => void
}) {
  const member =
    members.data?.find((candidate) => candidate.user.user_id === userId) ?? null
  if (!member)
    return (
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) =>
          members.loading ? (
            <Skeleton className="h-6 w-40 flex-1 rounded-full" />
          ) : (
            <h2
              id={surfaceTitleId("members", "workspace")}
              className={className}
            >
              {m.members_not_found()}
            </h2>
          )
        }
        action={<CloseButton onClick={onClose} />}
      />
    )
  return (
    <Body
      key={`${member.user.user_id}:${member.membership.role}`}
      networkId={networkId}
      member={member}
      ownership={ownership}
      viewer={viewer}
      self={member.user.user_id === viewerId}
      onClose={onClose}
      onOpenNode={onOpenNode}
    />
  )
}

function Body({
  networkId,
  member,
  ownership,
  viewer,
  self,
  onClose,
  onOpenNode,
}: {
  networkId: string
  member: AccountMember
  ownership: Read<NodeOwnershipList>
  viewer: AccountRole | null
  self: boolean
  onClose: () => void
  onOpenNode: (nodeId: string) => void
}) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const view = useNetwork()
  const { user, membership } = member
  const [confirming, setConfirming] = useState(false)

  const refresh = () =>
    Promise.all(
      [
        membersQuery(networkId),
        invitationsQuery(networkId),
        ownershipQuery(networkId),
        nodesQuery(networkId),
        sessionQuery(),
      ].map((query) =>
        queryClient.invalidateQueries({ queryKey: query.queryKey })
      )
    )

  const save = useMutation({
    mutationFn: (change: AccountMembershipChange) =>
      authed((base, token) =>
        saveAccountMember(base, token, networkId, {
          user_id: user.user_id,
          role: change,
        })
      ),
    onSuccess: (_, change) => {
      if (change === "remove") {
        setConfirming(false)
        onClose()
        toast.success(m.members_removed({ email: user.email }))
      } else toast.success(m.members_role_saved())
    },
    onError: toastMembershipFailure,
    onSettled: () => void refresh(),
  })

  const leave = useMutation({
    mutationFn: () =>
      authed((base, token) => leaveAccountNetwork(base, token, networkId)),
    onSuccess: async () => {
      setConfirming(false)
      toast.success(m.members_left())
      queryClient.removeQueries({ queryKey: ["network", networkId] })
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: networksQuery().queryKey }),
        queryClient.invalidateQueries({ queryKey: sessionQuery().queryKey }),
      ])
      await navigate({ to: "/" })
    },
    onError: toastMembershipFailure,
  })

  const devices = useMemo(() => {
    const owned: NodeSummary[] = []
    const unassigned: NodeSummary[] = []
    for (const entry of ownership.data?.nodes ?? []) {
      const node = view.byId.get(entry.node_id)
      if (!node || node.revoked_at !== null) continue
      if (entry.membership_id === membership.membership_id) owned.push(node)
      else if (entry.membership_id === null) unassigned.push(node)
    }
    const byName = (a: NodeSummary, b: NodeSummary) =>
      a.name.localeCompare(b.name) || a.id.localeCompare(b.id)
    return { owned: owned.sort(byName), unassigned: unassigned.sort(byName) }
  }, [ownership.data, view.byId, membership.membership_id])
  const devicesRead = ownership.data !== null && view.nodes.data !== undefined

  const removable = !self && canRemove(viewer, membership.role)
  const pending = save.isPending || leave.isPending

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("members", "workspace")} className={className}>
            {user.email}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
        meta={
          <div className="flex h-5 items-center gap-2 text-row text-muted-foreground">
            {roleLabel[membership.role]()}
            {self ? <YouMark /> : null}
          </div>
        }
      />
      <div className="min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
        <div className="flex max-w-xl flex-col gap-8">
          {canChangeRole(viewer) ? (
            <RoleForm
              role={membership.role}
              saving={save.isPending && save.variables !== "remove"}
              disabled={pending}
              onSave={(role) => save.mutate(role)}
            />
          ) : null}

          <FormSection
            title={
              devicesRead
                ? m.members_devices_count({ count: devices.owned.length })
                : m.members_devices()
            }
          >
            <NodeRows
              nodes={devicesRead ? devices.owned : null}
              empty={m.members_devices_empty()}
              runtimes={view.context.runtimes}
              now={view.now}
              onOpen={onOpenNode}
            />
          </FormSection>

          {viewer === "owner" && devices.unassigned.length > 0 ? (
            <Unassigned
              networkId={networkId}
              member={member}
              nodes={devices.unassigned}
              runtimes={view.context.runtimes}
              now={view.now}
              onClaimed={() =>
                void queryClient.invalidateQueries({
                  queryKey: ownershipQuery(networkId).queryKey,
                })
              }
            />
          ) : null}

          {self || removable ? (
            <div>
              <Button
                variant="destructive"
                className="h-9 rounded-full px-4"
                disabled={pending}
                onClick={() => setConfirming(true)}
              >
                {self ? <LogOutIcon /> : <UserMinusIcon />}
                {self ? m.members_leave() : m.members_remove()}
              </Button>
            </div>
          ) : null}
        </div>
      </div>

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={self ? m.members_leave() : m.members_remove()}
        confirmLabel={self ? m.members_leave() : m.members_remove()}
        pending={pending}
        onConfirm={() => (self ? leave.mutate() : save.mutate("remove"))}
      >
        <ul className="flex list-disc flex-col gap-1 pl-4">
          <li>
            {self
              ? m.members_leave_devices()
              : m.members_remove_devices({ email: user.email })}
          </li>
          <li>
            {self
              ? m.members_leave_invitations()
              : m.members_remove_invitations()}
          </li>
        </ul>
      </ConfirmDialog>
    </div>
  )
}

function RoleForm({
  role,
  saving,
  disabled,
  onSave,
}: {
  role: AccountRole
  saving: boolean
  disabled: boolean
  onSave: (role: AccountRole) => void
}) {
  const [chosen, setChosen] = useState(role)
  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (chosen !== role && !disabled) onSave(chosen)
  }
  return (
    <FormSection title={m.members_role()}>
      <form className="flex items-center gap-3" onSubmit={submit}>
        <SegmentedControl<AccountRole>
          label={m.members_role()}
          value={chosen}
          options={ROLES.map((value) => ({ value, label: roleLabel[value]() }))}
          onChange={setChosen}
        />
        <Button
          type="submit"
          className="h-9 rounded-full px-4"
          disabled={chosen === role || disabled}
          aria-busy={saving}
        >
          {saving ? <Spinner /> : null}
          {m.members_role_save()}
        </Button>
      </form>
    </FormSection>
  )
}

function NodeRows({
  nodes,
  empty,
  runtimes,
  now,
  onOpen,
  action,
}: {
  /** null while unread. */
  nodes: readonly NodeSummary[] | null
  empty: string
  runtimes: Runtimes
  now: number
  /** Rows open the node's workspace… */
  onOpen?: (nodeId: string) => void
  /** …or carry an action at their end instead. */
  action?: (node: NodeSummary) => ReactNode
}) {
  if (nodes === null)
    return (
      <div className="flex h-12 items-center gap-3 px-1">
        <Skeleton className="size-2 rounded-full" />
        <Skeleton className="h-3 w-32 rounded-full" />
      </div>
    )
  if (nodes.length === 0)
    return (
      <p className="flex h-12 items-center rounded-tile bg-fill-subtle px-3.5 text-row text-muted-foreground">
        {empty}
      </p>
    )
  return (
    <ul className="-mx-2">
      {nodes.map((node) => {
        const vitality = vitalityOf(node, runtimes, now)
        const facts = (
          <>
            <VitalityDot vitality={vitality} />
            <span className="min-w-0 flex-1 truncate text-row font-medium">
              {node.name}
            </span>
            <span className="shrink-0 font-mono text-meta text-muted-foreground">
              {node.mesh_ipv4}
            </span>
          </>
        )
        return (
          <li key={node.id}>
            {action ? (
              <div className="flex h-12 items-center gap-3 px-2">
                {facts}
                <span className="flex w-20 shrink-0 justify-end">
                  {action(node)}
                </span>
              </div>
            ) : (
              <button
                type="button"
                onClick={() => onOpen?.(node.id)}
                className="list-row h-12 gap-3 px-2"
              >
                {facts}
                <span className="flex w-20 shrink-0 items-center justify-end gap-1 text-meta text-muted-foreground">
                  {vitalityLabel[vitality]()}
                  <ChevronRightIcon className="size-4" />
                </span>
              </button>
            )}
          </li>
        )
      })}
    </ul>
  )
}

/** Nodes no membership owns yet, which an Owner may assign to this member (not undone here). */
function Unassigned({
  networkId,
  member,
  nodes,
  runtimes,
  now,
  onClaimed,
}: {
  networkId: string
  member: AccountMember
  nodes: readonly NodeSummary[]
  runtimes: Runtimes
  now: number
  onClaimed: () => void
}) {
  const [target, setTarget] = useState<NodeSummary | null>(null)
  const [open, setOpen] = useState(false)
  const claim = useMutation({
    mutationFn: (nodeId: string) =>
      authed((base, token) =>
        claimNodeOwnership(base, token, networkId, {
          node_id: nodeId,
          membership_id: member.membership.membership_id,
        })
      ),
    onSuccess: () => {
      setOpen(false)
      toast.success(m.members_claimed())
    },
    onError: (error) => {
      if (failureOf(error) === "conflict")
        toast.error(m.members_claim_conflict())
      else toastFailure(error)
    },
    onSettled: onClaimed,
  })

  return (
    <FormSection title={m.members_unassigned_count({ count: nodes.length })}>
      <NodeRows
        nodes={nodes}
        empty=""
        runtimes={runtimes}
        now={now}
        action={(node) => (
          <Button
            variant="secondary"
            size="sm"
            className="h-8 rounded-full px-3.5"
            disabled={claim.isPending}
            onClick={() => {
              setTarget(node)
              setOpen(true)
            }}
          >
            {m.members_claim()}
          </Button>
        )}
      />
      <GlassDialog
        open={open}
        onOpenChange={setOpen}
        title={m.members_claim_title()}
        busy={claim.isPending}
        footer={
          <>
            <Button
              variant="ghost"
              className="h-9 rounded-full px-4"
              disabled={claim.isPending}
              onClick={() => setOpen(false)}
            >
              {m.dialog_cancel()}
            </Button>
            <Button
              className="h-9 rounded-full px-4"
              disabled={claim.isPending}
              aria-busy={claim.isPending}
              onClick={() => {
                if (target) claim.mutate(target.id)
              }}
            >
              {claim.isPending ? <Spinner /> : null}
              {m.members_claim()}
            </Button>
          </>
        }
      >
        <p className="text-row text-muted-foreground">
          {m.members_claim_body({
            node: target?.name ?? "",
            email: member.user.email,
          })}
        </p>
      </GlassDialog>
    </FormSection>
  )
}
