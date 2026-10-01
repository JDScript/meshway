"use client"

import Link from "next/link"
import { usePathname } from "next/navigation"

import { useAccessHasDraft } from "@/components/access/draft-store"
import { useConnectionsHasDraft } from "@/components/connections/draft-store"
import { useRoutingHasDraft } from "@/components/routing/draft-store"
import { AccountMenu } from "@/components/shell/account-menu"
import { ContextSwitcher } from "@/components/shell/context-switcher"
import { isActive, navSections } from "@/components/shell/nav"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
  useSidebar,
} from "@/components/ui/sidebar"

export function AppSidebar({ controllerHost }: { controllerHost: string }) {
  const pathname = usePathname()
  const { isMobile, setOpenMobile } = useSidebar()
  const routingDraft = useRoutingHasDraft()
  const connectionsDraft = useConnectionsHasDraft()
  const accessDraft = useAccessHasDraft()
  const unsaved = (href: string) =>
    (href === "/routing" && routingDraft) ||
    (href === "/connections" && connectionsDraft) ||
    (href === "/access" && accessDraft)

  return (
    <Sidebar collapsible="icon" variant="inset">
      <SidebarHeader>
        <ContextSwitcher controllerHost={controllerHost} />
      </SidebarHeader>
      <SidebarContent>
        {navSections.map((section) => (
          <SidebarGroup key={section.label}>
            <SidebarGroupLabel>{section.label}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu aria-label={section.label}>
                {section.items.map((item) => {
                  const active = isActive(item.href, pathname)
                  return (
                    <SidebarMenuItem key={item.href}>
                      <SidebarMenuButton
                        isActive={active}
                        tooltip={item.label}
                        render={
                          <Link
                            href={item.href}
                            aria-current={active ? "page" : undefined}
                            onClick={() => {
                              if (isMobile) setOpenMobile(false)
                            }}
                          />
                        }
                      >
                        <item.icon />
                        <span>{item.label}</span>
                      </SidebarMenuButton>
                      {unsaved(item.href) ? (
                        <SidebarMenuBadge className="gap-1 text-[0.625rem] font-normal text-warning">
                          <span aria-hidden="true" className="size-1.5 rounded-full bg-warning" />
                          未保存
                        </SidebarMenuBadge>
                      ) : item.preview ? (
                        <SidebarMenuBadge className="text-[0.625rem] font-normal text-muted-foreground">
                          预览
                        </SidebarMenuBadge>
                      ) : null}
                    </SidebarMenuItem>
                  )
                })}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
      </SidebarContent>
      <SidebarFooter>
        <AccountMenu />
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  )
}
