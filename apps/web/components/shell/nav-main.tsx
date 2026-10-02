"use client"

import Link from "next/link"

import type { DraftKind, NavItem } from "@/components/shell/nav"
import { navMain, sectionDrafts } from "@/components/shell/nav"
import { useNetwork } from "@/components/shell/network-context"
import { SidebarGroup, SidebarMenu, SidebarMenuButton, SidebarMenuItem } from "@/components/ui/sidebar"

/**
 * The console's six places as plain links. Pages inside a section (tags,
 * DNS, diagnostics…) highlight their section; they are reached from that
 * section's own tabs and from ⌘K.
 */
export function NavMain({ active, drafts }: { active: NavItem | null; drafts: Record<DraftKind, boolean> }) {
  const { routes } = useNetwork()
  return (
    <SidebarGroup>
      <nav aria-label="控制台" className="contents">
        <SidebarMenu className="gap-0.5">
          {navMain.map((item) => {
            const current = active === item
            const unsaved = sectionDrafts(item).some((kind) => drafts[kind])
            return (
              <SidebarMenuItem key={item.segment}>
                <SidebarMenuButton
                  isActive={current}
                  tooltip={unsaved ? `${item.label}（有未保存的草稿）` : item.label}
                  render={<Link href={item.href(routes)} aria-current={current ? "page" : undefined} />}
                  className="h-9 font-medium data-active:shadow-xs"
                >
                  <item.icon aria-hidden="true" />
                  <span>{item.label}</span>
                  <UnsavedDot unsaved={unsaved} />
                </SidebarMenuButton>
              </SidebarMenuItem>
            )
          })}
        </SidebarMenu>
      </nav>
    </SidebarGroup>
  )
}

/** Amber dot; in the icon rail it sits on the icon's corner. */
function UnsavedDot({ unsaved }: { unsaved: boolean }) {
  if (!unsaved) return null
  return (
    <span className="ml-auto flex items-center group-data-[collapsible=icon]:absolute group-data-[collapsible=icon]:top-1 group-data-[collapsible=icon]:right-1" title="有未保存的草稿">
      <span aria-hidden="true" className="size-1.5 rounded-full bg-warning" />
      <span className="sr-only">（有未保存的草稿）</span>
    </span>
  )
}
