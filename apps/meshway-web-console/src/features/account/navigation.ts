import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/features/shell/history"

const PAGE = "/_authed/networks/$network/account" as const

/** Whether the URL is on the account page. */
export function useAccountRoute() {
  const current =
    useMatch({ from: PAGE, shouldThrow: false, select: () => true }) ?? false
  return { current }
}

/** Opening the account (from its menu) and closing it, under the shell's history rules. */
export function useAccountNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const { current } = useAccountRoute()
  return useMemo(
    () => ({
      open() {
        if (current) return
        void navigate({
          to: "/networks/$network/account",
          params: { network: networkId },
          ...history.open(false),
        })
      },
      close: history.leavePage,
    }),
    [navigate, history, networkId, current]
  )
}
