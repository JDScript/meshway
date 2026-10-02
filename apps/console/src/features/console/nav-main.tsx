import { Link } from "@tanstack/react-router"
import { GlobeIcon, ServerIcon, type LucideIcon } from "lucide-react"

import { SidebarGroup, SidebarMenu, SidebarMenuButton, SidebarMenuItem } from "@/components/ui/sidebar"
import { m } from "@/paraglide/messages.js"

type Destination = "/networks/$network/overview" | "/networks/$network/nodes"

export const NAV: { to: Destination; label: () => string; icon: LucideIcon }[] = [
  { to: "/networks/$network/overview", label: () => m.nav_overview(), icon: GlobeIcon },
  { to: "/networks/$network/nodes", label: () => m.nav_nodes(), icon: ServerIcon },
]

/** sidebar-07's main navigation, flat: only pages that exist are listed. */
export function NavMain({ networkId, active }: { networkId: string; active: Destination }) {
  return (
    <SidebarGroup>
      <SidebarMenu>
        {NAV.map((item) => (
          <SidebarMenuItem key={item.to}>
            <SidebarMenuButton
              tooltip={item.label()}
              isActive={item.to === active}
              render={<Link to={item.to} params={{ network: networkId }} />}
            >
              <item.icon />
              <span>{item.label()}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        ))}
      </SidebarMenu>
    </SidebarGroup>
  )
}
