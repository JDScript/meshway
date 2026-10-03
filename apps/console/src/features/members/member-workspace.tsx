import {
  claimNodeOwnership,
  leaveAccountNetwork,
  saveAccountMember,
  type AccountMember,
  type AccountMembershipChange,
  type AccountRole,
  type NodeOwnershipList,
  type NodeSummary,
} from "@meshway/control-client"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { LogOutIcon, UserMinusIcon, UserRoundPlusIcon } from "lucide-react"
import { useMemo, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { CloseButton } from "@/components/chrome/close-button"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { IconAction } from "@/components/chrome/icon-action"
import { RefList, RefRow } from "@/components/chrome/ref-row"
import { DangerRow, EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import {
  membersQuery,
  networkKey,
  networksQuery,
  nodesQuery,
  ownershipQuery,
  sessionQuery,
} from "@/features/network/queries"
import { useNetwork } from "@/features/network/use-network-view"
import {
  NodeRef,
  nodeVitality,
  type NodeContext,
} from "@/features/nodes/node-facts"
import { vitalityLabel } from "@/features/overview/labels"
import { VitalityDot } from "@/features/overview/status-dots"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { authed } from "@/lib/api"
import { toastFailureWith } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { YouMark } from "./member-list"
import { invitationsQuery } from "./queries"
import { canChangeRole, canRemove, ROLES, roleLabel } from "./roles"

type Read<T> = { data: T | null; loading: boolean }

/** A refused change on the last Owner says so; anything else as usual. */
const toastMembershipFailure = toastFailureWith({
  conflict: () => m.members_last_owner(),
})

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
      queryClient.removeQueries({ queryKey: networkKey(networkId) })
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
        <div className="flex max-w-workspace flex-col gap-8">
          {canChangeRole(viewer) ? (
            <RoleForm
              role={membership.role}
              saving={save.isPending && save.variables !== "remove"}
              disabled={pending}
              onSave={(role) => save.mutate(role)}
            />
          ) : null}

          <Section
            title={m.members_devices()}
            count={devicesRead ? devices.owned.length : null}
          >
            {!devicesRead ? (
              <Skeleton className="h-12 w-full rounded-tile" />
            ) : devices.owned.length === 0 ? (
              <EmptyLine>{m.members_devices_empty()}</EmptyLine>
            ) : (
              <RefList>
                {devices.owned.map((node) => (
                  <NodeRef
                    key={node.id}
                    nodeId={node.id}
                    context={view.context}
                    onOpen={onOpenNode}
                  />
                ))}
              </RefList>
            )}
          </Section>

          {viewer === "owner" && devices.unassigned.length > 0 ? (
            <Unassigned
              networkId={networkId}
              member={member}
              nodes={devices.unassigned}
              context={view.context}
              onClaimed={() =>
                void queryClient.invalidateQueries({
                  queryKey: ownershipQuery(networkId).queryKey,
                })
              }
            />
          ) : null}

          {self || removable ? (
            <Section title={self ? m.members_leaving() : m.members_removal()}>
              <DangerRow
                fact={m.members_devices_fact({
                  count: devicesRead ? String(devices.owned.length) : "—",
                })}
                action={
                  <Button
                    variant="destructive"
                    className="h-9 rounded-full px-4"
                    disabled={pending}
                    onClick={() => setConfirming(true)}
                  >
                    {self ? <LogOutIcon /> : <UserMinusIcon />}
                    {self ? m.members_leave() : m.members_remove()}
                  </Button>
                }
              />
            </Section>
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
        effects={
          self
            ? [m.members_leave_devices(), m.members_leave_invitations()]
            : [
                m.members_remove_devices({ email: user.email }),
                m.members_remove_invitations(),
              ]
        }
      />
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
    <Section title={m.members_role()}>
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
    </Section>
  )
}

/** Nodes no membership owns yet, which an Owner may assign to this member (not undone here). */
function Unassigned({
  networkId,
  member,
  nodes,
  context,
  onClaimed,
}: {
  networkId: string
  member: AccountMember
  nodes: readonly NodeSummary[]
  context: NodeContext
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
    onError: toastFailureWith({ conflict: () => m.members_claim_conflict() }),
    onSettled: onClaimed,
  })

  return (
    <Section title={m.members_unassigned()} count={nodes.length}>
      <RefList>
        {nodes.map((node) => {
          const vitality = nodeVitality(node, context)
          return (
            <RefRow
              key={node.id}
              lead={<VitalityDot vitality={vitality} />}
              title={node.name}
              detail={node.mesh_ipv4}
              state={vitalityLabel[vitality]()}
              action={
                <IconAction
                  label={m.members_claim_node({ node: node.name })}
                  icon={UserRoundPlusIcon}
                  disabled={claim.isPending}
                  onClick={() => {
                    setTarget(node)
                    setOpen(true)
                  }}
                />
              }
            />
          )
        })}
      </RefList>
      <ConfirmDialog
        open={open}
        onOpenChange={setOpen}
        title={m.members_claim_title()}
        confirmLabel={m.members_claim()}
        tone="default"
        pending={claim.isPending}
        onConfirm={() => {
          if (target) claim.mutate(target.id)
        }}
        effects={[
          m.members_claim_body({
            node: target?.name ?? "",
            email: member.user.email,
          }),
        ]}
      />
    </Section>
  )
}
