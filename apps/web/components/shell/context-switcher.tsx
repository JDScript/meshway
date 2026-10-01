"use client"

import { ChevronsUpDownIcon, NetworkIcon } from "lucide-react"

import { BrandMark } from "@/components/shell/brand"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar"

/**
 * Identifies the one network this console manages. The controller has a
 * single mesh, so this is context information, not a pretend network switch.
 */
export function ContextSwitcher({ controllerHost }: { controllerHost: string }) {
  const { isMobile } = useSidebar()

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <SidebarMenuButton
                size="lg"
                aria-label="当前网络"
                className="data-popup-open:bg-sidebar-accent"
              />
            }
          >
            <BrandMark className="size-8" />
            <div className="grid min-w-0 flex-1 text-left leading-tight">
              <span className="truncate text-sm font-semibold tracking-tight">Meshway</span>
              <span className="truncate font-mono text-[0.6875rem] text-muted-foreground">
                {controllerHost}
              </span>
            </div>
            <ChevronsUpDownIcon className="ml-auto text-muted-foreground" />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="w-(--anchor-width) min-w-64"
            side={isMobile ? "bottom" : "right"}
            align="start"
            sideOffset={6}
          >
            <DropdownMenuGroup>
              <DropdownMenuLabel>当前网络</DropdownMenuLabel>
              <div className="flex items-start gap-2 px-2 py-1.5 text-sm">
                <NetworkIcon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 break-all font-mono text-xs">{controllerHost}</span>
              </div>
              <p className="px-2 pt-1 pb-1.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
                此控制台管理 controller 的单个网络。
              </p>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  )
}
