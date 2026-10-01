"use client"

import Link from "next/link"
import { usePathname } from "next/navigation"
import { useEffect, type ReactNode } from "react"

import { useAccessHasDraft } from "@/components/access/draft-store"
import { useConnectionsHasDraft } from "@/components/connections/draft-store"
import { NetworkWorkspace } from "@/components/network/network-workspace"
import { useRoutingHasDraft } from "@/components/routing/draft-store"
import { AppSidebar } from "@/components/shell/app-sidebar"
import { CommandMenu } from "@/components/shell/command-menu"
import { activeNavItem, type DraftKind } from "@/components/shell/nav"
import { useNetwork } from "@/components/shell/network-context"
import type { NetworkChoices } from "@/components/shell/network-switcher"
import { ThemeToggle } from "@/components/shell/theme-toggle"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import { Separator } from "@/components/ui/separator"
import { SidebarInset, SidebarProvider, SidebarTrigger, useSidebar } from "@/components/ui/sidebar"

/**
 * Console frame for one network. The workspace binds the network's own
 * draft stores: unsaved work and its CAS base never follow the user into
 * another network's editor, and come back when they return.
 */
export function ConsoleShell({
  networkId,
  networks,
  controllerHost,
  sidebarOpen,
  children,
}: {
  networkId: string
  networks: NetworkChoices
  controllerHost: string
  /** Desktop sidebar state remembered by the sidebar's own cookie. */
  sidebarOpen: boolean
  children: ReactNode
}) {
  const current = networks.ok ? networks.networks.find((network) => network.networkId === networkId) : undefined
  return (
    <NetworkWorkspace networkId={networkId} name={current?.name ?? null}>
      <SidebarProvider defaultOpen={sidebarOpen}>
        <ConsoleFrame networks={networks} controllerHost={controllerHost}>
          {children}
        </ConsoleFrame>
      </SidebarProvider>
    </NetworkWorkspace>
  )
}

export function useDrafts(): Record<DraftKind, boolean> {
  return {
    access: useAccessHasDraft(),
    routing: useRoutingHasDraft(),
    connections: useConnectionsHasDraft(),
  }
}

function ConsoleFrame({
  networks,
  controllerHost,
  children,
}: {
  networks: NetworkChoices
  controllerHost: string
  children: ReactNode
}) {
  const drafts = useDrafts()
  const pathname = usePathname()
  const { setOpenMobile } = useSidebar()
  // Any navigation (links, ⌘K, network switch, history) closes the mobile drawer.
  useEffect(() => {
    setOpenMobile(false)
  }, [pathname, setOpenMobile])

  return (
    <>
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-card focus:px-3 focus:py-2 focus:text-sm focus:shadow"
      >
        跳到主要内容
      </a>
      <AppSidebar networks={networks} controllerHost={controllerHost} drafts={drafts} />
      <SidebarInset className="min-w-0">
        <ConsoleHeader />
        <div id="main" tabIndex={-1} className="flex min-w-0 flex-1 flex-col outline-none">
          {children}
        </div>
      </SidebarInset>
    </>
  )
}

/**
 * sidebar-07's inset header: trigger, separator and breadcrumb, 4rem tall
 * and 3rem while the sidebar is an icon rail. Sticky so search and the
 * sidebar stay reachable on long pages; the row shares the page column.
 */
function ConsoleHeader() {
  const pathname = usePathname()
  const { routes, name } = useNetwork()
  const { isMobile, open } = useSidebar()
  const active = activeNavItem(pathname, routes.base)
  const detail = active ? pathname.replace(/\/$/, "") !== active.item.href(routes).split("?")[0] : false

  return (
    <header
      data-slot="console-topbar"
      className="sticky top-0 z-30 flex h-16 shrink-0 items-center border-b bg-header backdrop-blur-md transition-[width,height] ease-linear group-has-data-[collapsible=icon]/sidebar-wrapper:h-12"
    >
      <div className="console-column flex items-center gap-2">
        <SidebarTrigger className="-ml-1" aria-label={isMobile ? "打开导航" : open ? "收起导航" : "展开导航"} aria-keyshortcuts="Meta+B" />
        <Separator orientation="vertical" className="mr-2 data-vertical:h-4 data-vertical:self-auto" />
        <Breadcrumb aria-label="当前位置" className="min-w-0 flex-1">
          <BreadcrumbList className="flex-nowrap text-[0.8125rem] whitespace-nowrap">
            <BreadcrumbItem className="min-w-0 shrink">
              <BreadcrumbLink render={<Link href={routes.overview} />} className="max-w-48 truncate">
                {name ?? "网络"}
              </BreadcrumbLink>
            </BreadcrumbItem>
            {active?.section ? (
              <>
                <BreadcrumbSeparator className="hidden lg:block" />
                <BreadcrumbItem className="hidden shrink-0 lg:inline-flex">{active.section.label}</BreadcrumbItem>
              </>
            ) : null}
            {active && active.item.segment !== "" ? (
              <>
                <BreadcrumbSeparator />
                <BreadcrumbItem className="min-w-0 shrink-0">
                  {detail ? (
                    <BreadcrumbLink render={<Link href={active.item.href(routes)} />} className="truncate">
                      {active.item.label}
                    </BreadcrumbLink>
                  ) : (
                    <BreadcrumbPage className="truncate">{active.item.label}</BreadcrumbPage>
                  )}
                </BreadcrumbItem>
              </>
            ) : null}
          </BreadcrumbList>
        </Breadcrumb>
        <div data-slot="console-topbar-actions" className="flex shrink-0 items-center gap-1.5">
          <CommandMenu />
          <ThemeToggle />
        </div>
      </div>
    </header>
  )
}
