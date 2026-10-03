import {
  changeMemberRole,
  type MembershipRole,
  type MemberSummary,
  type NetworkMembersResponse,
  type NetworkSummary,
} from "@meshway/control-client"
import {
  useMutation,
  useQueryClient,
  type InfiniteData,
  type UseInfiniteQueryResult,
} from "@tanstack/react-query"
import { LogOutIcon, UserMinusIcon } from "lucide-react"
import { useState, type FormEvent } from "react"
import { toast } from "sonner"

import { BusyLabel } from "@/components/chrome/busy-label"
import { CloseButton } from "@/components/chrome/close-button"
import { DangerRow, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { networkQuery } from "@/features/network/queries"
import { roleLabel, ROLES } from "@/features/network/roles"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { authed, client } from "@/lib/api"
import { toastFailureWith } from "@/lib/failure"
import { calendarDate } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { YouMark } from "./member-list"
import { joinLinksQuery, membersQuery } from "./queries"
import { LeaveNetworkDialog, RemoveMemberDialog } from "./removal"

type MembersRead = UseInfiniteQueryResult<InfiniteData<NetworkMembersResponse>>

/**
 * One member in full: their role, when they joined, and removal (for
 * oneself, leaving). Only what `actions` allows is offered; the controller
 * checks again.
 */
export function MemberWorkspace({
  networkId,
  network,
  membershipId,
  members,
  viewerId,
  onClose,
}: {
  networkId: string
  network: NetworkSummary | null
  membershipId: string
  members: MembersRead
  viewerId: string | null
  onClose: () => void
}) {
  const member =
    members.data?.pages
      .flatMap((page) => page.items)
      .find((candidate) => candidate.membershipId === membershipId) ?? null
  if (!member)
    return (
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) =>
          members.data === undefined && !members.isError ? (
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
      // A new version starts the form from what the controller now reports.
      key={`${member.membershipId}:${member.version}`}
      networkId={networkId}
      network={network}
      member={member}
      self={member.userId === viewerId}
      onClose={onClose}
    />
  )
}

/** A role change the controller refused because the member moved meanwhile. */
const ROLE_FAILURE = { version_conflict: () => m.members_changed() }

function Body({
  networkId,
  network,
  member,
  self,
  onClose,
}: {
  networkId: string
  network: NetworkSummary | null
  member: MemberSummary
  self: boolean
  onClose: () => void
}) {
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)

  const save = useMutation({
    mutationFn: (role: MembershipRole) =>
      authed((token) =>
        changeMemberRole(
          client,
          { networkId, membershipId: member.membershipId },
          { expectedVersion: member.version, role },
          { token }
        )
      ),
    onSuccess: () =>
      toast.success(m.members_role_saved(), { id: "member-role" }),
    onError: toastFailureWith(ROLE_FAILURE),
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
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("members", "workspace")} className={className}>
            {member.email}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
        meta={
          <div className="flex h-5 items-center gap-2 text-row text-muted-foreground">
            {roleLabel[member.role]()}
            {self ? <YouMark /> : null}
          </div>
        }
      />
      <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
        <div className="flex max-w-workspace flex-col gap-8">
          {member.actions.changeRole ? (
            <RoleForm
              role={member.role}
              roles={member.actions.assignableRoles}
              saving={save.isPending}
              onSave={(role) => save.mutate(role)}
            />
          ) : null}

          <Section title={m.members_facts()}>
            <div className="grid gap-2 sm:grid-cols-2">
              {member.actions.changeRole ? null : (
                <Tile label={m.members_role()}>{roleLabel[member.role]()}</Tile>
              )}
              <Tile label={m.members_joined_at()}>
                {calendarDate(member.joinedAt) ?? "—"}
              </Tile>
            </div>
          </Section>

          {member.actions.remove ? (
            <Section title={self ? m.members_leaving() : m.members_removal()}>
              <DangerRow
                fact={self ? m.members_leave_fact() : m.members_remove_fact()}
                action={
                  <Button
                    variant="destructive"
                    className="h-9 rounded-full px-4"
                    disabled={save.isPending}
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

      {self ? (
        <LeaveNetworkDialog
          networkId={networkId}
          networkName={network?.name ?? ""}
          open={confirming}
          onOpenChange={setConfirming}
        />
      ) : (
        <RemoveMemberDialog
          networkId={networkId}
          member={member}
          open={confirming}
          onOpenChange={setConfirming}
          onRemoved={onClose}
        />
      )}
    </div>
  )
}

/** The role, chosen among those the controller says this account may assign. */
function RoleForm({
  role,
  roles,
  saving,
  onSave,
}: {
  role: MembershipRole
  roles: readonly MembershipRole[]
  saving: boolean
  onSave: (role: MembershipRole) => void
}) {
  const [chosen, setChosen] = useState(role)
  const options = ROLES.filter(
    (value) => value === role || roles.includes(value)
  )
  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (chosen !== role && !saving) onSave(chosen)
  }
  return (
    <Section title={m.members_role()}>
      <form className="flex flex-wrap items-center gap-3" onSubmit={submit}>
        <SegmentedControl<MembershipRole>
          label={m.members_role()}
          value={chosen}
          options={options.map((value) => ({
            value,
            label: roleLabel[value](),
          }))}
          onChange={setChosen}
        />
        <Button
          type="submit"
          className="h-9 rounded-full px-4"
          disabled={chosen === role || saving}
          aria-busy={saving}
        >
          <BusyLabel busy={saving}>{m.members_role_save()}</BusyLabel>
        </Button>
      </form>
    </Section>
  )
}
