import { ChevronsUpDownIcon } from "lucide-react"
import type { Ref } from "react"

import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem, useSidebar } from "@/components/ui/sidebar"
import { NetworkMenuItems } from "@/features/network/network-menu"
import type { LinkSummary } from "@/features/overview/model"
import { m } from "@/paraglide/messages.js"

import { Orb } from "./orb"

/**
 * sidebar-07's team switcher, as the network switcher. The docked globe
 * takes the logo's place: it is a separate button that leads back to the
 * globe, and stays when the sidebar collapses to icons.
 */
export function NetworkSwitcher({
  networkId,
  name,
  nodes,
  links,
  orbRef,
  onOrb,
}: {
  networkId: string
  name: string | null
  nodes: number | null
  links: LinkSummary | null
  orbRef: Ref<HTMLButtonElement>
  onOrb: () => void
}) {
  const { isMobile } = useSidebar()
  const known = links?.kind === "known" ? links : null
  return (
    <SidebarMenu>
      <SidebarMenuItem className="flex items-center gap-1.5">
        <Orb ref={orbRef} links={links} onClick={onOrb} className="size-12 group-data-[collapsible=icon]:size-8" />
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <SidebarMenuButton
                size="lg"
                className="min-w-0 flex-1 data-open:bg-sidebar-accent data-open:text-sidebar-accent-foreground group-data-[collapsible=icon]:hidden"
              />
            }
          >
            <div className="grid flex-1 text-left text-sm leading-tight">
              <span className="truncate font-medium">{name ?? "\u00a0"}</span>
              <span className="truncate text-xs text-muted-foreground tabular-nums">
                {m.console_summary({
                  nodes: nodes === null ? "—" : String(nodes),
                  confirmed: known ? String(known.confirmed) : "—",
                  total: known ? String(known.total) : "—",
                })}
              </span>
            </div>
            <ChevronsUpDownIcon className="ml-auto" />
          </DropdownMenuTrigger>
          <DropdownMenuContent className="min-w-60" align="start" side={isMobile ? "bottom" : "right"} sideOffset={4}>
            <NetworkMenuItems current={networkId} />
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  )
}
