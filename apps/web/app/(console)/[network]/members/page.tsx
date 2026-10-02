import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { MembersWorkspace } from "@/components/members/members-workspace"
import { PageBody } from "@/components/shell/page-header"
import { fetchConsoleIdentity } from "@/lib/account-session"
import { roleIn } from "@/lib/account-model"
import { controllerUrl, fetchInvitations, fetchMembers, fetchNodeOwnership, fetchNodes } from "@/lib/controller"
import { canManageMembers, type MemberActor } from "@/lib/members-model"
import { requireNetwork } from "@/lib/network-scope"

export const metadata: Metadata = { title: "成员" }

export default async function MembersPage({
  params,
  searchParams,
}: {
  params: Promise<{ network: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [{ networkId }, identity, query] = await Promise.all([requireNetwork(params), fetchConsoleIdentity(), searchParams])
  const actor: MemberActor =
    identity.kind === "bootstrap"
      ? { kind: "bootstrap" }
      : { kind: "user", userId: identity.kind === "user" ? identity.userId : "", role: roleIn(identity, networkId) }
  const manage = canManageMembers(actor)
  // Invitations are refused (403) to Members, so they are only read for those who manage.
  const [members, ownership, nodes, invitations] = await Promise.all([
    fetchMembers(networkId),
    fetchNodeOwnership(networkId),
    fetchNodes(networkId),
    manage ? fetchInvitations(networkId) : Promise.resolve(null),
  ])

  if (!members.ok) {
    return (
      <PageBody>
        <h1 className="text-[1.375rem] font-semibold tracking-tight">成员</h1>
        <ControllerUnavailable url={controllerUrl()} error={members.error} />
      </PageBody>
    )
  }

  return (
    <PageBody>
      <MembersWorkspace
        actor={actor}
        identityError={identity.kind === "user-unverified" ? identity.error : null}
        members={members.data.members}
        fetchedAt={members.fetchedAt}
        ownership={ownership.ok ? { ok: true, entries: ownership.data.nodes } : { ok: false, error: ownership.error }}
        nodes={nodes.ok ? { ok: true, list: nodes.data.nodes } : { ok: false, error: nodes.error }}
        invitations={invitations === null ? null : invitations.ok ? { ok: true, list: invitations.data.invitations } : { ok: false, error: invitations.error }}
        openInvite={query.invite === "1" && manage}
      />
    </PageBody>
  )
}
