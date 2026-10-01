"use client"

import { ChevronRightIcon } from "lucide-react"
import Link from "next/link"
import { useState } from "react"

import type { DraftKind, NavItem, NavSection } from "@/components/shell/nav"
import { navMain } from "@/components/shell/nav"
import { useNetwork } from "@/components/shell/network-context"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import {
  SidebarGroup,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  useSidebar,
} from "@/components/ui/sidebar"

/**
 * sidebar-07 `NavMain` with the console's real pages: plain links, and
 * collapsible sections whose sub-pages are links. In the icon rail a
 * section's icon goes to its first page, since its sub-menu is hidden there.
 */
export function NavMain({ active, drafts }: { active: NavItem | undefined; drafts: Record<DraftKind, boolean> }) {
  return (
    <SidebarGroup>
      <SidebarGroupLabel>网络</SidebarGroupLabel>
      <nav aria-label="控制台" className="contents">
        <SidebarMenu>
          {navMain.map((entry) =>
            entry.kind === "link" ? (
              <SidebarMenuItem key={entry.item.segment}>
                <NavLink item={entry.item} active={active === entry.item} unsaved={entry.item.draft ? drafts[entry.item.draft] : false} />
              </SidebarMenuItem>
            ) : (
              <NavSectionItem key={entry.section.key} section={entry.section} active={active} drafts={drafts} />
            )
          )}
        </SidebarMenu>
      </nav>
    </SidebarGroup>
  )
}

function NavLink({ item, active, unsaved }: { item: NavItem; active: boolean; unsaved: boolean }) {
  const { routes } = useNetwork()
  return (
    <SidebarMenuButton
      isActive={active}
      tooltip={unsaved ? `${item.label}（有未保存的草稿）` : item.label}
      render={<Link href={item.href(routes)} aria-current={active ? "page" : undefined} />}
    >
      <item.icon aria-hidden="true" />
      <span>{item.label}</span>
      <UnsavedDot unsaved={unsaved} />
    </SidebarMenuButton>
  )
}

function NavSectionItem({ section, active, drafts }: { section: NavSection; active: NavItem | undefined; drafts: Record<DraftKind, boolean> }) {
  const { routes } = useNetwork()
  const { state, isMobile } = useSidebar()
  const containsActive = section.items.some((item) => item === active)
  const unsaved = section.items.some((item) => item.draft && drafts[item.draft])
  const listed = section.items.filter((item) => !item.hidden)
  // Opening follows the current page until the user toggles; entering the
  // section again (e.g. via ⌘K) opens it again.
  const [toggle, setToggle] = useState<{ open: boolean; whileActive: boolean } | null>(null)
  if (toggle !== null && toggle.whileActive !== containsActive) setToggle(null)
  const open = toggle?.open ?? containsActive
  const first = listed[0]

  if (state === "collapsed" && !isMobile && first) {
    return (
      <SidebarMenuItem>
        <SidebarMenuButton
          isActive={containsActive}
          tooltip={unsaved ? `${section.label}（有未保存的草稿）` : section.label}
          render={<Link href={first.href(routes)} aria-current={containsActive ? "page" : undefined} />}
        >
          <section.icon aria-hidden="true" />
          <span>{section.label}</span>
          <UnsavedDot unsaved={unsaved} />
        </SidebarMenuButton>
      </SidebarMenuItem>
    )
  }

  return (
    <Collapsible
      open={open}
      onOpenChange={(next) => setToggle({ open: next, whileActive: containsActive })}
      className="group/collapsible"
      render={<SidebarMenuItem />}
    >
      <CollapsibleTrigger render={<SidebarMenuButton tooltip={section.label} />}>
        <section.icon aria-hidden="true" />
        <span>{section.label}</span>
        {!open ? <UnsavedDot unsaved={unsaved} /> : null}
        <ChevronRightIcon aria-hidden="true" className="ml-auto transition-transform duration-200 group-data-open/collapsible:rotate-90" />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <SidebarMenuSub>
          {listed.map((item) => {
            const current = active === item
            const draft = item.draft ? drafts[item.draft] : false
            return (
              <SidebarMenuSubItem key={item.segment}>
                <SidebarMenuSubButton
                  isActive={current}
                  title={item.description}
                  render={<Link href={item.href(routes)} aria-current={current ? "page" : undefined} />}
                >
                  <span>{item.label}</span>
                  {item.badge ? (
                    <span aria-hidden="true" className="ml-1 rounded border border-sidebar-border px-1 text-[0.625rem] leading-4 text-sidebar-foreground/60">
                      {item.badge}
                    </span>
                  ) : null}
                  <UnsavedDot unsaved={draft} />
                </SidebarMenuSubButton>
              </SidebarMenuSubItem>
            )
          })}
        </SidebarMenuSub>
      </CollapsibleContent>
    </Collapsible>
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
