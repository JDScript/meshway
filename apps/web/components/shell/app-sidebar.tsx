"use client"

import { usePathname } from "next/navigation"

import { activeNavItem, type DraftKind } from "@/components/shell/nav"
import { NavMain } from "@/components/shell/nav-main"
import { NavUser } from "@/components/shell/nav-user"
import { useNetwork } from "@/components/shell/network-context"
import { NetworkSwitcher, type NetworkChoices } from "@/components/shell/network-switcher"
import { Sidebar, SidebarContent, SidebarFooter, SidebarHeader, SidebarRail } from "@/components/ui/sidebar"

/**
 * shadcn sidebar-07 `AppSidebar`: network switcher (the block's team
 * switcher) in the header, the console's pages in `NavMain`, the session in
 * `NavUser`. All data is the controller's; the block's sample data is gone.
 */
export function AppSidebar({
  networks,
  controllerHost,
  drafts,
  collapsible,
}: {
  networks: NetworkChoices
  controllerHost: string
  drafts: Record<DraftKind, boolean>
  /** "offcanvas" while the overview is an immersive map; otherwise the icon rail. */
  collapsible: "icon" | "offcanvas"
}) {
  const pathname = usePathname()
  const { routes } = useNetwork()
  const active = activeNavItem(pathname, routes.base)?.item

  return (
    <Sidebar collapsible={collapsible}>
      <SidebarHeader>
        <NetworkSwitcher networks={networks} controllerHost={controllerHost} />
      </SidebarHeader>
      <SidebarContent>
        <NavMain active={active} drafts={drafts} />
      </SidebarContent>
      <SidebarFooter>
        <NavUser />
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  )
}
