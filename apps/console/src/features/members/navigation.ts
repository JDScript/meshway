import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/features/shell/history"

import type { MembersView } from "./url"

const LIST = "/_authed/networks/$network/members" as const
const WORKSPACE = "/_authed/networks/$network/members/$member" as const

/** The members page as the URL has it. */
export function useMembersRoute() {
  const current =
    useMatch({ from: LIST, shouldThrow: false, select: () => true }) ?? false
  const view: MembersView =
    useMatch({
      from: LIST,
      shouldThrow: false,
      select: (match) => match.search.view ?? "members",
    }) ?? "members"
  const member =
    useMatch({
      from: WORKSPACE,
      shouldThrow: false,
      select: (match) => match.params.member,
    }) ?? null
  return { current, view, member }
}

/** Every move on the members page, under the shell's history rules. */
export function useMemberNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const { view, member: openId } = useMembersRoute()

  return useMemo(() => {
    const params = { network: networkId }
    const search = (next: MembersView) => ({
      view: next === "members" ? undefined : next,
    })
    return {
      openMember(membershipId: string) {
        void navigate({
          to: "/networks/$network/members/$member",
          params: { ...params, member: membershipId },
          search: search(view),
          ...history.open(openId !== null),
        })
      },
      closeMember() {
        history.close(
          () =>
            void navigate({
              to: "/networks/$network/members",
              params,
              search: search(view),
              replace: true,
            })
        )
      },
      /** The panel's view, with whatever is open kept open. */
      setView(next: MembersView) {
        if (openId)
          void navigate({
            to: "/networks/$network/members/$member",
            params: { ...params, member: openId },
            search: search(next),
            ...history.keep,
          })
        else
          void navigate({
            to: "/networks/$network/members",
            params,
            search: search(next),
            ...history.keep,
          })
      },
    }
  }, [navigate, history, networkId, view, openId])
}
