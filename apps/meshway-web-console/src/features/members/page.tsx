import type { NetworkSummary } from "@meshway/control-client"
import { useInfiniteQuery, useQuery } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useEffect } from "react"

import { sessionQuery } from "@/features/auth/queries"
import { PageFrame } from "@/features/shell/page-frame"
import { useNow } from "@/hooks/use-now"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { failureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { MemberList } from "./member-list"
import { MemberWorkspace } from "./member-workspace"
import { useMemberNavigation, useMembersRoute } from "./navigation"
import { joinLinksQuery, membersQuery } from "./queries"

/**
 * The members page: the members (or the join links), and one member's
 * workspace over them. It exists only for those who may manage members;
 * when that changes while it is open, the page gives way to the overview.
 */
export function MembersPage({
  networkId,
  network,
}: {
  networkId: string
  network: NetworkSummary | null
}) {
  const navigate = useNavigate()
  const route = useMembersRoute()
  const current = route.current
  const nav = useMemberNavigation(networkId)
  const allowed = network?.actions.manageMembers ?? false
  const session = useQuery(sessionQuery())
  const now = useNow(30_000)
  const members = useInfiniteQuery({
    ...membersQuery(networkId),
    enabled: route.current && allowed,
  })
  const links = useInfiniteQuery({
    ...joinLinksQuery(networkId),
    enabled: route.current && allowed,
  })

  useEffect(() => {
    if (current && network && !allowed)
      void navigate({
        to: "/networks/$network/overview",
        params: { network: networkId },
        replace: true,
      })
  }, [current, network, allowed, navigate, networkId])

  useReadFailureToast(
    "members",
    route.current && members.isError && !members.isFetchNextPageError,
    failureText(members.error) ?? m.members_failed()
  )
  useReadFailureToast(
    "join-links",
    route.current &&
      route.view === "links" &&
      links.isError &&
      !links.isFetchNextPageError,
    failureText(links.error) ?? m.members_links_failed()
  )

  const viewerId = session.data?.user.id ?? null
  return (
    <PageFrame
      networkId={networkId}
      page="members"
      current={route.current && allowed}
      panel={
        <MemberList
          networkId={networkId}
          joinLinkRoles={network?.actions.joinLinkRoles ?? []}
          view={route.view}
          members={members}
          links={links}
          viewerId={viewerId}
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
            network={network}
            membershipId={route.member}
            members={members}
            viewerId={viewerId}
            onClose={nav.closeMember}
          />
        ) : null
      }
      onCloseWorkspace={nav.closeMember}
    />
  )
}
