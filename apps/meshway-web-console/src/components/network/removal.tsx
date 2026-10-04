import {
  leaveNetwork,
  removeMember,
  type MemberSummary,
  type RemovalImpactResponse,
} from "@meshway/control-client"
import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { toast } from "sonner"

import { authed, client, codeOf } from "@/api/client"
import { failureText, toastFailureWith } from "@/api/failure"
import { sessionQuery } from "@/api/queries/auth"
import {
  joinLinksQuery,
  leaveImpactQuery,
  membersQuery,
  removalImpactQuery,
} from "@/api/queries/members"
import { networkKey, networksQuery } from "@/api/queries/network"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { forgetNetwork } from "@/lib/remembered-network"
import { m } from "@/paraglide/messages.js"

// Removing a member and leaving a network do the same to a membership: it
// ends, the member's current User-identity devices are deleted for good
// (Tag devices stay), and the join links it issued and nobody used are
// revoked. What is deleted is read when the confirmation opens; the change
// is sent against the version and revision of that read, so anything
// changed meanwhile is refused rather than overwritten.

/** The facts of a removal, as its impact was read. */
function effects(
  impact: UseQueryResult<RemovalImpactResponse>,
  who: { self: true; network: string } | { self: false; email: string }
): string[] {
  const membership = who.self
    ? m.members_leave_effect({ network: who.network })
    : m.members_remove_effect({ email: who.email })
  const devices = impact.data
    ? impact.data.nodeCount === 0
      ? m.members_devices_none()
      : m.members_devices_deleted({ count: String(impact.data.nodeCount) })
    : impact.isError
      ? (failureText(impact.error) ?? m.members_impact_failed())
      : m.members_impact_reading()
  const links = who.self ? m.members_leave_links() : m.members_remove_links()
  return [membership, devices, links]
}

/** A removal or leave the controller refused because something moved. */
const CONFLICT = {
  version_conflict: () => m.members_changed(),
  revision_conflict: () => m.members_changed(),
}

/** Confirms removing `member`, then removes them. */
export function RemoveMemberDialog({
  networkId,
  member,
  open,
  onOpenChange,
  onRemoved,
}: {
  networkId: string
  member: MemberSummary
  open: boolean
  onOpenChange: (open: boolean) => void
  onRemoved: () => void
}) {
  const queryClient = useQueryClient()
  const impact = useQuery({
    ...removalImpactQuery(networkId, member.membershipId),
    enabled: open,
  })
  const refresh = () =>
    Promise.all(
      [membersQuery(networkId), joinLinksQuery(networkId)].map((query) =>
        queryClient.invalidateQueries({ queryKey: query.queryKey })
      )
    )

  const remove = useMutation({
    mutationFn: (read: RemovalImpactResponse) =>
      authed((token) =>
        removeMember(
          client,
          { networkId, membershipId: member.membershipId },
          { expectedVersion: read.version, expectedRevision: read.revision },
          { token }
        )
      ),
    onSuccess: () => {
      onOpenChange(false)
      toast.success(m.members_removed({ email: member.email }))
      onRemoved()
    },
    onError: (error) => {
      toastFailureWith(CONFLICT)(error)
      if (moved(error)) void impact.refetch()
    },
    onSettled: () => void refresh(),
  })

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title={m.members_remove()}
      confirmLabel={m.members_remove()}
      pending={remove.isPending}
      ready={impact.data !== undefined && !impact.isFetching}
      onConfirm={() => {
        if (impact.data) remove.mutate(impact.data)
      }}
      effects={effects(impact, { self: false, email: member.email })}
    />
  )
}

/** Confirms leaving the network, then leaves it and opens another (or none). */
export function LeaveNetworkDialog({
  networkId,
  networkName,
  open,
  onOpenChange,
}: {
  networkId: string
  networkName: string
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const session = useQuery(sessionQuery())
  const impact = useQuery({ ...leaveImpactQuery(networkId), enabled: open })

  const leave = useMutation({
    mutationFn: (read: RemovalImpactResponse) =>
      authed((token) =>
        leaveNetwork(
          client,
          { networkId },
          { expectedVersion: read.version, expectedRevision: read.revision },
          { token }
        )
      ),
    onSuccess: async () => {
      onOpenChange(false)
      toast.success(m.members_left({ network: networkName }))
      if (session.data) forgetNetwork(session.data.user.id, networkId)
      await queryClient.invalidateQueries({
        queryKey: networksQuery().queryKey,
      })
      await navigate({ to: "/", replace: true })
      queryClient.removeQueries({ queryKey: networkKey(networkId) })
    },
    onError: (error) => {
      toastFailureWith(CONFLICT)(error)
      if (moved(error)) void impact.refetch()
    },
  })

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title={m.members_leave()}
      confirmLabel={m.members_leave()}
      pending={leave.isPending}
      ready={impact.data !== undefined && !impact.isFetching}
      onConfirm={() => {
        if (impact.data) leave.mutate(impact.data)
      }}
      effects={effects(impact, { self: true, network: networkName })}
    />
  )
}

const moved = (error: unknown) => {
  const code = codeOf(error)
  return code === "version_conflict" || code === "revision_conflict"
}
