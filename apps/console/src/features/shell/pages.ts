import { useMatches, useNavigate } from "@tanstack/react-router"
import { useCallback } from "react"
import {
  GlobeIcon,
  ServerIcon,
  Settings2Icon,
  UsersIcon,
  ShieldIcon,
  type LucideIcon,
} from "lucide-react"

import { m } from "@/paraglide/messages.js"

// The pages of a network, the one list the dock, the network switcher and
// the shell read. A page's own surfaces live in `features/<page>/page.tsx`.

export const PAGES = {
  overview: { route: "/networks/$network/overview", panel: false },
  nodes: { route: "/networks/$network/nodes", panel: true },
  members: { route: "/networks/$network/members", panel: true },
  policy: { route: "/networks/$network/policy", panel: true },
  settings: { route: "/networks/$network/settings", panel: true },
  account: { route: "/networks/$network/account", panel: false },
} as const satisfies Record<
  string,
  { route: `/networks/$network/${string}`; panel: boolean }
>

export type PageId = keyof typeof PAGES

/** The dock, in order. Pages reached elsewhere (the account, from its menu) are not in it. */
export const DOCK: readonly {
  page: PageId
  icon: LucideIcon
  label: () => string
}[] = [
  { page: "overview", icon: GlobeIcon, label: () => m.nav_overview() },
  { page: "nodes", icon: ServerIcon, label: () => m.nav_nodes() },
  { page: "members", icon: UsersIcon, label: () => m.members_title() },
  { page: "policy", icon: ShieldIcon, label: () => m.policy_nav() },
  { page: "settings", icon: Settings2Icon, label: () => m.nav_settings() },
]

declare module "@tanstack/react-router" {
  interface StaticDataRouteOption {
    /** On a page's route: which page it is. */
    page?: PageId
    /** On a route that opens a workspace over its page. */
    workspace?: true
  }
}

/** The page the URL is on (the overview until a page route matches). */
export function useCurrentPage(): PageId {
  return useMatches({
    select: (matches) =>
      matches.reduce<PageId>(
        (page, match) => match.staticData.page ?? page,
        "overview"
      ),
  })
}

/** Whether the URL opens a workspace. */
export function useWorkspaceOpen(): boolean {
  return useMatches({
    select: (matches) =>
      matches.some((match) => match.staticData.workspace === true),
  })
}

/** Another network: the same page, nothing open. */
export function useSwitchNetwork(
  networkId: string,
  page: PageId
): (target: string) => void {
  const navigate = useNavigate()
  return useCallback(
    (target: string) => {
      if (target !== networkId)
        void navigate({ to: PAGES[page].route, params: { network: target } })
    },
    [navigate, networkId, page]
  )
}
