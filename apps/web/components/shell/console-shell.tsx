"use client"

import { MenuIcon, PanelLeftIcon } from "lucide-react"
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
import { Button } from "@/components/ui/button"
import { SidebarProvider, useSidebar } from "@/components/ui/sidebar"

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
      <div data-slot="sidebar-inset" className="relative flex min-w-0 flex-1 flex-col bg-background">
        <ConsoleTopbar />
        <main id="main" className="flex min-w-0 flex-1 flex-col">
          {children}
        </main>
      </div>
    </>
  )
}

/** Only context and global actions: where am I, search, theme. */
function ConsoleTopbar() {
  const pathname = usePathname()
  const { routes, name } = useNetwork()
  const { isMobile, open, toggleSidebar } = useSidebar()
  const active = activeNavItem(pathname, routes.base)
  const detail = active && pathname.replace(/\/$/, "") !== active.item.href(routes).split("?")[0]

  return (
    <header data-slot="console-topbar" className="sticky top-0 z-30 border-b bg-header backdrop-blur-md">
      <div className="mx-auto flex h-14 w-full max-w-[76rem] items-center gap-2 px-4 md:px-8">
        <Button
          variant="ghost"
          size="icon"
          data-sidebar="trigger"
          className="-ml-1.5 text-muted-foreground hover:text-foreground"
          aria-label={isMobile ? "打开导航" : open ? "收起导航" : "展开导航"}
          aria-keyshortcuts="Meta+B"
          onClick={toggleSidebar}
        >
          <MenuIcon className="md:hidden" />
          <PanelLeftIcon className="hidden md:block" />
        </Button>
        <span aria-hidden="true" className="h-4 w-px bg-border" />
        <nav aria-label="当前位置" className="min-w-0 flex-1">
          <ol className="flex min-w-0 items-center gap-1.5 text-[0.8125rem] text-muted-foreground">
            <li className="min-w-0 truncate md:hidden">
              <span className="font-medium text-foreground">{name ?? "网络"}</span>
            </li>
            {active ? (
              <>
                <li aria-hidden="true" className="md:hidden">/</li>
                {active.group?.label ? (
                  <>
                    <li className="hidden shrink-0 md:block">{active.group.label}</li>
                    <li aria-hidden="true" className="hidden md:block">/</li>
                  </>
                ) : null}
                <li className="min-w-0 truncate">
                  {detail ? (
                    <Link href={active.item.href(routes)} className="hover:text-foreground">{active.item.label}</Link>
                  ) : (
                    <span aria-current="page" className="font-medium text-foreground md:font-normal md:text-muted-foreground">{active.item.label}</span>
                  )}
                </li>
              </>
            ) : null}
          </ol>
        </nav>
        <div data-slot="console-topbar-actions" className="flex shrink-0 items-center gap-1.5">
          <CommandMenu />
          <ThemeToggle />
        </div>
      </div>
    </header>
  )
}
