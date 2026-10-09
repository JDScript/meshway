import type { NetworkActions } from "@meshway/control-client"
import { useMatches, useNavigate } from "@tanstack/react-router"
import {
  FileSlidersIcon,
  GlobeIcon,
  ServerIcon,
  Settings2Icon,
  UsersIcon,
  WaypointsIcon,
  type LucideIcon,
} from "lucide-react"
import { useCallback } from "react"

import { m } from "@/paraglide/messages.js"

// The pages of a network, the one list the dock, the network switcher and
// the shell read. Each page's route draws its own surfaces.
// Only pages with operations behind them are here (docs/decisions.md).

export const PAGES = {
  overview: { route: "/networks/$network/overview", panel: false },
  topology: { route: "/networks/$network/topology", panel: false },
  nodes: { route: "/networks/$network/nodes", panel: true },
  members: { route: "/networks/$network/members", panel: true },
  configuration: { route: "/networks/$network/configuration", panel: true },
  settings: { route: "/networks/$network/settings", panel: true },
  account: { route: "/networks/$network/account", panel: false },
} as const satisfies Record<
  string,
  { route: `/networks/$network/${string}`; panel: boolean }
>

/** The page the configuration draft belongs to: the dock marks it while the draft holds edits, wherever they were made. */
export const DRAFT_PAGE: PageId = "configuration"

export type PageId = keyof typeof PAGES
export type CanvasPage = "overview" | "topology"
export const isCanvasPage = (page: PageId): page is CanvasPage =>
  page === "overview" || page === "topology"

export type DockItem = {
  page: PageId
  icon: LucideIcon
  label: () => string
  /** Offered only when the network's actions allow it. */
  shown?: (actions: NetworkActions) => boolean
}

/** The dock, in order. Pages reached elsewhere (the account, from its menu) are not in it. */
const DOCK: readonly DockItem[] = [
  { page: "overview", icon: GlobeIcon, label: () => m.nav_overview() },
  { page: "topology", icon: WaypointsIcon, label: () => m.overview_topology() },
  { page: "nodes", icon: ServerIcon, label: () => m.nodes_title() },
  {
    page: "members",
    icon: UsersIcon,
    label: () => m.members_title(),
    shown: (actions) => actions.manageMembers,
  },
  {
    page: "configuration",
    icon: FileSlidersIcon,
    label: () => m.configuration_title(),
  },
  { page: "settings", icon: Settings2Icon, label: () => m.nav_settings() },
]

/** The dock for what this account may do here; only the overview until the network is read. */
export function dockFor(actions: NetworkActions | null): readonly DockItem[] {
  return DOCK.filter((item) =>
    item.shown ? actions !== null && item.shown(actions) : true
  )
}

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

/**
 * Another network: the same page, nothing open. A page that network may
 * not offer (its members) redirects to its overview.
 */
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
