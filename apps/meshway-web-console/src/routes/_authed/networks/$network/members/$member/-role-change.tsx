import {
  changeMemberRole,
  type MembershipRole,
  type MemberSummary,
  type RoleChangeImpactResponse,
} from "@meshway/control-client"
import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from "@tanstack/react-query"
import { toast } from "sonner"

import { authed, client, codeOf } from "@/api/client"
import { failureText, toastFailureWith } from "@/api/failure"
import {
  joinLinksQuery,
  membersQuery,
  roleChangeImpactQuery,
} from "@/api/queries/members"
import { networkQuery } from "@/api/queries/network"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { roleLabel } from "@/lib/roles"
import { m } from "@/paraglide/messages.js"

// Changing a member's role revokes the join links they issued, nobody used
// and that have not expired, when the new role may not issue them. What is
// revoked is read when the confirmation opens; the change is sent against
// the membership version of that read, so a member changed meanwhile is
// refused rather than overwritten.

/** The facts of a role change, as its impact was read. */
function effects(
  impact: UseQueryResult<RoleChangeImpactResponse>,
  member: MemberSummary,
  role: MembershipRole,
  self: boolean
): string[] {
  const from = roleLabel[member.role]()
  const to = roleLabel[role]()
  const change = self
    ? m.members_role_change_self({ from, to })
    : m.members_role_change_effect({ email: member.email, from, to })
  const links = impact.data
    ? impact.data.joinLinkCount === 0
      ? m.members_role_change_links_none()
      : self
        ? m.members_role_change_links_self({
            count: impact.data.joinLinkCount,
          })
        : m.members_role_change_links({ count: impact.data.joinLinkCount })
    : impact.isError
      ? (failureText(impact.error) ?? m.members_role_change_failed())
      : m.members_role_change_reading()
  return [change, links]
}

/** A role change the controller refused because the member moved meanwhile. */
const CONFLICT = { version_conflict: () => m.members_changed() }

/** Confirms changing `member` to `role`, then changes it. */
export function RoleChangeDialog({
  networkId,
  member,
  role,
  self,
  open,
  onOpenChange,
}: {
  networkId: string
  member: MemberSummary
  role: MembershipRole
  self: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const queryClient = useQueryClient()
  const impact = useQuery({
    ...roleChangeImpactQuery(networkId, member.membershipId, role),
    enabled: open,
  })

  const change = useMutation({
    mutationFn: (read: RoleChangeImpactResponse) =>
      authed((token) =>
        changeMemberRole(
          client,
          { networkId, membershipId: member.membershipId },
          { expectedVersion: read.version, role: read.role },
          { token }
        )
      ),
    onSuccess: () => {
      onOpenChange(false)
      toast.success(m.members_role_saved(), { id: "member-role" })
    },
    onError: (error) => {
      toastFailureWith(CONFLICT)(error)
      if (codeOf(error) === "version_conflict") void impact.refetch()
    },
    // Roles change what this account may do (its own) and which links stay valid.
    onSettled: () =>
      Promise.all(
        [
          membersQuery(networkId),
          joinLinksQuery(networkId),
          networkQuery(networkId),
        ].map((query) =>
          queryClient.invalidateQueries({ queryKey: query.queryKey })
        )
      ),
  })

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title={m.members_role_change()}
      confirmLabel={m.members_role_save()}
      tone="default"
      pending={change.isPending}
      ready={impact.data !== undefined && !impact.isFetching}
      onConfirm={() => {
        if (impact.data) change.mutate(impact.data)
      }}
      effects={effects(impact, member, role, self)}
    />
  )
}
