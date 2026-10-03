import type { NetworkSummary } from "@meshway/control-client"
import { useInfiniteQuery, useQuery } from "@tanstack/react-query"
import { useMatch, useNavigate } from "@tanstack/react-router"
import { useEffect } from "react"

import { sessionQuery } from "@/features/auth/queries"
import { PageFrame } from "@/features/shell/page-frame"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { failureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { MemberList } from "./member-list"
import { membersQuery } from "./queries"

const PAGE = "/_authed/networks/$network/members" as const

/**
 * The members page: who is in the network, read only. It exists only for
 * those who may manage members; when that changes while it is open, the
 * page gives way to the overview.
 */
export function MembersPage({
  networkId,
  network,
}: {
  networkId: string
  network: NetworkSummary | null
}) {
  const navigate = useNavigate()
  const current =
    useMatch({ from: PAGE, shouldThrow: false, select: () => true }) ?? false
  const allowed = network?.actions.manageMembers ?? false
  const session = useQuery(sessionQuery())
  const members = useInfiniteQuery({
    ...membersQuery(networkId),
    enabled: current && allowed,
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
    current && members.isError && !members.isFetchNextPageError,
    failureText(members.error) ?? m.members_failed()
  )

  return (
    <PageFrame
      networkId={networkId}
      page="members"
      current={current && allowed}
      panel={
        <MemberList
          members={members}
          viewerId={session.data?.user.id ?? null}
        />
      }
      workspace={null}
      onCloseWorkspace={() => undefined}
    />
  )
}
