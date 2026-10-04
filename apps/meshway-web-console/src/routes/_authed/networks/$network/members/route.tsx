import { useInfiniteQuery, useQuery } from "@tanstack/react-query"
import { createFileRoute, redirect, useNavigate } from "@tanstack/react-router"
import { useEffect } from "react"

import { failureText } from "@/api/failure"
import { sessionQuery } from "@/api/queries/auth"
import { joinLinksQuery, membersQuery } from "@/api/queries/members"
import { networkQuery, useNetworkSummary } from "@/api/queries/network"
import { PageFrame } from "@/components/surface/page-frame"
import { useNow } from "@/hooks/use-now"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { m } from "@/paraglide/messages.js"

import { MemberList } from "./-member-list"
import { useMemberNavigation, useMembersRoute } from "./-navigation"
import { zMembersSearch } from "./-search"
import { MemberWorkspace } from "./$member/-workspace"

/** The members, for those who may manage them, and one member over them (its child route). */
export const Route = createFileRoute("/_authed/networks/$network/members")({
  staticData: { page: "members" },
  validateSearch: zMembersSearch,
  loader: async ({ context: { queryClient }, params }) => {
    // The network's own route says when it cannot be read.
    const network = await queryClient
      .ensureQueryData(networkQuery(params.network))
      .catch(() => null)
    if (network && !network.network.actions.manageMembers)
      throw redirect({
        to: "/networks/$network/overview",
        params,
        replace: true,
      })
  },
  component: MembersPage,
})

/**
 * The members page: the members (or the join links), and one member's
 * workspace over them. It exists only for those who may manage members;
 * when that changes while it is open, the page gives way to the overview.
 */
function MembersPage() {
  const { network: networkId } = Route.useParams()
  const network = useNetworkSummary(networkId)
  const navigate = useNavigate()
  const route = useMembersRoute()
  const nav = useMemberNavigation(networkId)
  const allowed = network?.actions.manageMembers ?? false
  const session = useQuery(sessionQuery())
  const now = useNow(30_000)
  const members = useInfiniteQuery({
    ...membersQuery(networkId),
    enabled: allowed,
  })
  const links = useInfiniteQuery({
    ...joinLinksQuery(networkId),
    enabled: allowed,
  })

  useEffect(() => {
    if (network && !allowed)
      void navigate({
        to: "/networks/$network/overview",
        params: { network: networkId },
        replace: true,
      })
  }, [network, allowed, navigate, networkId])

  useReadFailureToast(
    "members",
    members.isError && !members.isFetchNextPageError,
    failureText(members.error) ?? m.members_failed()
  )
  useReadFailureToast(
    "join-links",
    route.view === "links" && links.isError && !links.isFetchNextPageError,
    failureText(links.error) ?? m.members_links_failed()
  )

  const viewerId = session.data?.user.id ?? null
  // Until the network is read, and once it is not offered.
  if (!allowed) return null
  return (
    <PageFrame
      networkId={networkId}
      page="members"
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
