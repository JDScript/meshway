import { useQuery } from "@tanstack/react-query"

import { sessionQuery } from "@/features/network/queries"
import { useNetworkRole } from "@/features/network/role"
import { useNodeNavigation } from "@/features/nodes/navigation"
import { PageFrame } from "@/features/shell/page-frame"
import { useNow } from "@/lib/use-now"
import { useReadFailureToast } from "@/lib/use-read-failure-toast"
import { m } from "@/paraglide/messages.js"

import { MemberList } from "./member-list"
import { MemberWorkspace } from "./member-workspace"
import { useMemberNavigation, useMembersRoute } from "./navigation"
import { invitationsQuery, membersQuery, ownershipQuery } from "./queries"
import { grantableRoles } from "./roles"

/** The members page: the members (or invitations), and one member's workspace over them. */
export function MembersPage({ networkId }: { networkId: string }) {
  const route = useMembersRoute()
  const nav = useMemberNavigation(networkId)
  const nodes = useNodeNavigation(networkId)
  const session = useQuery(sessionQuery())
  const viewer = useNetworkRole(networkId)
  const manages = grantableRoles(viewer).length > 0
  const now = Math.floor(useNow(30_000) / 1000)

  const members = useQuery({
    ...membersQuery(networkId),
    enabled: route.current,
  })
  const invitations = useQuery({
    ...invitationsQuery(networkId),
    enabled: route.current && manages && route.view === "invitations",
  })
  const ownership = useQuery({
    ...ownershipQuery(networkId),
    enabled: route.current && route.member !== null,
  })
  useReadFailureToast(
    "members",
    route.current && members.isError,
    m.members_failed()
  )
  useReadFailureToast(
    "invitations",
    route.current && invitations.isError,
    m.members_invitations_failed()
  )
  useReadFailureToast(
    "ownership",
    route.current && route.member !== null && ownership.isError,
    m.members_devices_failed()
  )

  const read = <T,>(query: {
    data: T | undefined
    isPending: boolean
    isError: boolean
  }) => ({
    data: query.data ?? null,
    loading: query.data === undefined && !query.isError,
    failed: query.data === undefined && query.isError,
  })

  return (
    <PageFrame
      networkId={networkId}
      page="members"
      current={route.current}
      panel={
        <MemberList
          networkId={networkId}
          viewer={viewer}
          viewerId={session.data?.user.user_id ?? null}
          view={route.view}
          members={{ ...read(members), data: members.data?.members ?? null }}
          invitations={{
            ...read(invitations),
            data: invitations.data?.invitations ?? null,
          }}
          openId={route.member}
          now={now}
          onView={nav.setView}
          onOpen={nav.openMember}
        />
      }
      workspace={
        route.member ? (
          <MemberWorkspace
            networkId={networkId}
            userId={route.member}
            members={{
              data: members.data?.members ?? null,
              loading: members.data === undefined && !members.isError,
            }}
            ownership={{
              data: ownership.data ?? null,
              loading: ownership.data === undefined && !ownership.isError,
            }}
            viewer={viewer}
            viewerId={session.data?.user.user_id ?? null}
            onClose={nav.closeMember}
            onOpenNode={(nodeId) => nodes.openNode(nodeId)}
          />
        ) : null
      }
      onCloseWorkspace={nav.closeMember}
    />
  )
}
