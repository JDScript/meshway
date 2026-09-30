"use client"

import { ChevronsUpDownIcon, NetworkIcon, UsersIcon } from "lucide-react"

import { BrandMark } from "@/components/shell/brand"
import { Badge } from "@/components/ui/badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
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
 * single mesh and no team or membership model yet, so the menu names the
 * actual controller and shows team management as not available — it never
 * offers a switch that would pretend to isolate anything.
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
              <DropdownMenuLabel>网络</DropdownMenuLabel>
              <DropdownMenuRadioGroup value="default">
                <DropdownMenuRadioItem value="default" closeOnClick>
                  <NetworkIcon />
                  <div className="grid min-w-0 leading-tight">
                    <span>默认网络</span>
                    <span className="truncate font-mono text-[0.625rem] text-muted-foreground">
                      {controllerHost}
                    </span>
                  </div>
                </DropdownMenuRadioItem>
              </DropdownMenuRadioGroup>
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              <DropdownMenuLabel>团队</DropdownMenuLabel>
              <DropdownMenuItem disabled>
                <UsersIcon />
                <span className="flex-1">团队与多网络</span>
                <Badge variant="outline">规划中</Badge>
              </DropdownMenuItem>
              <p className="px-2 pt-1 pb-1.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
                当前 controller 管理单个网络，尚无团队、成员或网络隔离。
              </p>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  )
}
