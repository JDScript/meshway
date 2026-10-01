"use client"

import Link from "next/link"
import { usePathname } from "next/navigation"
import type { ReactNode } from "react"

import { useAccessHasDraft } from "@/components/access/draft-store"
import { useConnectionsHasDraft } from "@/components/connections/draft-store"
import { NetworkWorkspace } from "@/components/network/network-workspace"
import { useRoutingHasDraft } from "@/components/routing/draft-store"
import { AccountMenu } from "@/components/shell/account-menu"
import { BrandMark } from "@/components/shell/brand"
import { CommandMenu } from "@/components/shell/command-menu"
import { activeTab, navTabs, type DraftKind } from "@/components/shell/nav"
import { useNetwork } from "@/components/shell/network-context"
import { NetworkSwitcher, type NetworkChoices } from "@/components/shell/network-switcher"
import { ThemeToggle } from "@/components/shell/theme-toggle"
import { cn } from "@/lib/utils"

/**
 * Console frame for one network. The workspace binds the network's own
 * draft stores: unsaved work and its CAS base never follow the user into
 * another network's editor, and come back when they return.
 */
export function ConsoleShell({
  networkId,
  networks,
  controllerHost,
  children,
}: {
  networkId: string
  networks: NetworkChoices
  controllerHost: string
  children: ReactNode
}) {
  const current = networks.ok ? networks.networks.find((network) => network.networkId === networkId) : undefined
  return (
    <NetworkWorkspace networkId={networkId} name={current?.name ?? null}>
      <div className="flex min-h-svh flex-col">
        <ConsoleHeader networks={networks} controllerHost={controllerHost} />
        <main id="main" className="flex min-w-0 flex-1 flex-col">
          {children}
        </main>
      </div>
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

function ConsoleHeader({ networks, controllerHost }: { networks: NetworkChoices; controllerHost: string }) {
  const pathname = usePathname()
  const { routes } = useNetwork()
  const current = activeTab(pathname, routes.base)
  const drafts = useDrafts()

  return (
    <header className="sticky top-0 z-30 border-b bg-header backdrop-blur-md supports-backdrop-filter:bg-header">
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-card focus:px-3 focus:py-2 focus:text-sm focus:shadow"
      >
        跳到主要内容
      </a>
      <div className="mx-auto flex h-14 max-w-[76rem] items-center gap-2 px-4 md:px-8">
        <Link href={routes.overview} className="flex shrink-0 items-center gap-2 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring/40" aria-label="Meshway 概览">
          <BrandMark className="size-7" />
          <span className="hidden text-[0.9375rem] font-semibold tracking-tight sm:inline">Meshway</span>
        </Link>
        <span aria-hidden="true" className="mx-1 hidden h-5 w-px rotate-12 bg-border sm:block" />
        <NetworkSwitcher networks={networks} controllerHost={controllerHost} />
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
          <CommandMenu />
          <span className="hidden sm:contents">
            <ThemeToggle />
          </span>
          <AccountMenu />
        </div>
      </div>
      <nav aria-label="控制台" className="mx-auto max-w-[76rem] px-2 md:px-6">
        <ul className="-mb-px flex gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {navTabs.map((tab) => {
            const active = current?.key === tab.key
            const unsaved = tab.children?.some((item) => item.draft && drafts[item.draft]) ?? false
            return (
              <li key={tab.key} className="shrink-0">
                <Link
                  href={tab.href(routes)}
                  aria-current={active ? "page" : undefined}
                  className={cn(
                    "relative flex h-10 items-center gap-1.5 rounded-t-md px-2.5 text-[0.8125rem] font-medium text-muted-foreground transition-colors outline-none hover:text-foreground focus-visible:bg-accent",
                    "after:absolute after:inset-x-2 after:bottom-0 after:h-0.5 after:rounded-full after:bg-transparent",
                    active && "text-foreground after:bg-brand"
                  )}
                >
                  <tab.icon className="size-4 opacity-80" aria-hidden="true" />
                  {tab.label}
                  {unsaved ? (
                    <span className="flex items-center" title="有未保存的草稿">
                      <span aria-hidden="true" className="size-1.5 rounded-full bg-warning" />
                      <span className="sr-only">（有未保存的草稿）</span>
                    </span>
                  ) : null}
                </Link>
              </li>
            )
          })}
        </ul>
      </nav>
    </header>
  )
}
