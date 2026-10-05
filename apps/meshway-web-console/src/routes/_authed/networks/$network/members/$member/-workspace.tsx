import {
  type MembershipRole,
  type MemberSummary,
  type NetworkMembersResponse,
  type NetworkSummary,
} from "@meshway/control-client"
import type {
  InfiniteData,
  UseInfiniteQueryResult,
} from "@tanstack/react-query"
import { LogOutIcon, UserMinusIcon } from "lucide-react"
import { useState, type FormEvent } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { DangerRow, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import {
  LeaveNetworkDialog,
  RemoveMemberDialog,
} from "@/components/network/removal"
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { calendarDate } from "@/lib/format"
import { roleLabel, ROLES } from "@/lib/roles"
import { m } from "@/paraglide/messages.js"

import { YouMark } from "../-member-list"
import { RoleChangeDialog } from "./-role-change"

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
  const [confirming, setConfirming] = useState(false)
  // The role last chosen, kept while its confirmation closes.
  const [changing, setChanging] = useState<{
    role: MembershipRole
    open: boolean
  } | null>(null)

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
              onSave={(role) => setChanging({ role, open: true })}
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

      {changing ? (
        <RoleChangeDialog
          networkId={networkId}
          member={member}
          role={changing.role}
          self={self}
          open={changing.open}
          onOpenChange={(open) => setChanging({ ...changing, open })}
        />
      ) : null}
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
  onSave,
}: {
  role: MembershipRole
  roles: readonly MembershipRole[]
  onSave: (role: MembershipRole) => void
}) {
  const [chosen, setChosen] = useState(role)
  const options = ROLES.filter(
    (value) => value === role || roles.includes(value)
  )
  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (chosen !== role) onSave(chosen)
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
          disabled={chosen === role}
        >
          {m.members_role_save()}
        </Button>
      </form>
    </Section>
  )
}
