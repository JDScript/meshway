"use client"

import Link from "next/link"
import type { NetworkSnapshot } from "@meshway/control-client"
import { usePathname } from "next/navigation"

import { CapabilityPopover } from "@/components/shell/capability-popover"
import { CommandMenu } from "@/components/shell/command-menu"
import { currentItem, currentSection } from "@/components/shell/nav"
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
import { SidebarTrigger } from "@/components/ui/sidebar"
import type { ControllerResult } from "@/lib/controller"

export function SiteHeader({ network }: { network?: ControllerResult<NetworkSnapshot> }) {
  const pathname = usePathname()
  const item = currentItem(pathname)
  const section = currentSection(pathname)
  // Detail pages (e.g. /nodes/<id>) sit one level below their section.
  const detail = item !== undefined && item.href !== "/" && pathname !== item.href

  return (
    <header className="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-2 border-b bg-background/85 px-3 backdrop-blur supports-backdrop-filter:bg-background/70 md:rounded-t-xl md:px-4">
      <SidebarTrigger className="-ml-1" aria-label="切换侧边栏" />
      <Separator orientation="vertical" className="mr-1 data-vertical:h-4 data-vertical:self-center" />
      <Breadcrumb className="min-w-0">
        <BreadcrumbList className="flex-nowrap">
          {network?.ok ? (
            <>
              <BreadcrumbItem className="hidden min-w-0 lg:inline-flex">
                <span className="max-w-36 truncate text-muted-foreground" title={network.data.name}>{network.data.name}</span>
              </BreadcrumbItem>
              <BreadcrumbSeparator className="hidden lg:inline-flex" />
            </>
          ) : null}
          <BreadcrumbItem className="hidden sm:inline-flex">
            <span className="text-muted-foreground">{section?.label ?? "Meshway"}</span>
          </BreadcrumbItem>
          <BreadcrumbSeparator className="hidden sm:inline-flex" />
          {detail ? (
            <>
              <BreadcrumbItem className="min-w-0">
                <BreadcrumbLink render={<Link href={item.href} />}>{item.label}</BreadcrumbLink>
              </BreadcrumbItem>
              <BreadcrumbSeparator />
              <BreadcrumbItem className="min-w-0">
                <BreadcrumbPage className="truncate">详情</BreadcrumbPage>
              </BreadcrumbItem>
            </>
          ) : (
            <BreadcrumbItem className="min-w-0">
              <BreadcrumbPage className="truncate">{item?.label ?? "控制台"}</BreadcrumbPage>
            </BreadcrumbItem>
          )}
        </BreadcrumbList>
      </Breadcrumb>
      <div className="ml-auto flex items-center gap-1.5">
        <CommandMenu />
        <div className="hidden sm:block">
          <CapabilityPopover />
        </div>
        <ThemeToggle />
      </div>
    </header>
  )
}
