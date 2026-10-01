"use client"

import Link from "next/link"
import { usePathname } from "next/navigation"

import { AccountMenu } from "@/components/shell/account-menu"
import { BrandMark } from "@/components/shell/brand"
import { activeNavItem, navGroups, settingsItem, type DraftKind, type NavItem } from "@/components/shell/nav"
import { useNetwork } from "@/components/shell/network-context"
import { NetworkSwitcher, type NetworkChoices } from "@/components/shell/network-switcher"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
  useSidebar,
} from "@/components/ui/sidebar"

/**
 * Primary navigation. The network is the first thing in the sidebar: every
 * page below it belongs to that network, like a tailnet in Tailscale.
 */
export function AppSidebar({
  networks,
  controllerHost,
  drafts,
}: {
  networks: NetworkChoices
  controllerHost: string
  drafts: Record<DraftKind, boolean>
}) {
  const pathname = usePathname()
  const { routes } = useNetwork()
  const active = activeNavItem(pathname, routes.base)?.item

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader className="gap-3 px-3 pt-3 group-data-[collapsible=icon]:px-2">
        <Link
          href={routes.overview}
          aria-label="Meshway 概览"
          className="flex h-8 items-center gap-2 rounded-md px-1 outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring group-data-[collapsible=icon]:px-0"
        >
          <BrandMark className="size-7" />
          <span className="text-[0.9375rem] font-semibold tracking-tight group-data-[collapsible=icon]:hidden">Meshway</span>
        </Link>
        <NetworkSwitcher networks={networks} controllerHost={controllerHost} />
      </SidebarHeader>
      <SidebarContent className="gap-1 pt-2">
        <nav aria-label="控制台" className="contents">
          {navGroups.map((group) => (
            <SidebarGroup key={group.key} className="px-3 group-data-[collapsible=icon]:px-2">
              {group.label ? (
                <SidebarGroupLabel className="h-7 px-2 text-[0.75rem] font-medium text-sidebar-foreground/55">{group.label}</SidebarGroupLabel>
              ) : null}
              <SidebarGroupContent>
                <SidebarMenu>
                  {group.items.map((item) => (
                    <NavLink key={item.segment} item={item} active={active === item} unsaved={item.draft ? drafts[item.draft] : false} />
                  ))}
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
          ))}
          <SidebarGroup className="mt-auto px-3 group-data-[collapsible=icon]:px-2">
            <SidebarMenu>
              <NavLink item={settingsItem} active={active === settingsItem} unsaved={false} />
            </SidebarMenu>
          </SidebarGroup>
        </nav>
      </SidebarContent>
      <SidebarFooter className="border-t border-sidebar-border px-3 py-2 group-data-[collapsible=icon]:px-2">
        <AccountMenu />
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  )
}

function NavLink({ item, active, unsaved }: { item: NavItem; active: boolean; unsaved: boolean }) {
  const { routes } = useNetwork()
  const { isMobile, setOpenMobile } = useSidebar()
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={active}
        tooltip={unsaved ? `${item.label}（有未保存的草稿）` : item.label}
        className="relative h-8 gap-2.5 px-2 text-[0.875rem] text-sidebar-foreground/80 data-active:text-sidebar-foreground [&_svg]:text-sidebar-foreground/60 data-active:[&_svg]:text-brand"
        render={
          <Link
            href={item.href(routes)}
            aria-current={active ? "page" : undefined}
            title={item.description}
            onClick={() => {
              if (isMobile) setOpenMobile(false)
            }}
          />
        }
      >
        <item.icon aria-hidden="true" />
        {unsaved ? (
          <span aria-hidden="true" className="absolute top-1 right-1 hidden size-1.5 rounded-full bg-warning group-data-[collapsible=icon]:block" />
        ) : null}
        <span className="flex min-w-0 flex-1 items-center gap-1.5">
          <span className="truncate">{item.label}</span>
          {item.badge ? (
            <span aria-hidden="true" className="rounded border border-sidebar-border px-1 text-[0.6875rem] leading-4 font-normal text-sidebar-foreground/55">
              {item.badge}
            </span>
          ) : null}
          {unsaved ? (
            <span className="ml-auto flex items-center" title="有未保存的草稿">
              <span aria-hidden="true" className="size-1.5 rounded-full bg-warning" />
              <span className="sr-only">（有未保存的草稿）</span>
            </span>
          ) : null}
        </span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  )
}
