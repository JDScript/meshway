"use client"

import { BookOpenIcon, ChevronsUpDownIcon, LogOutIcon, MonitorIcon, MoonIcon, SunIcon } from "lucide-react"
import { useTheme } from "next-themes"
import { useTransition } from "react"

import { useDraftVault } from "@/components/network/network-workspace"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
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
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem, useSidebar } from "@/components/ui/sidebar"
import { logout } from "@/lib/auth-actions"
import { docsUrl } from "@/lib/config"

const themes = [
  { value: "system", label: "跟随系统", icon: MonitorIcon },
  { value: "light", label: "浅色", icon: SunIcon },
  { value: "dark", label: "深色", icon: MoonIcon },
] as const

/**
 * sidebar-07 `NavUser`. The console has one administrator token session
 * today (user accounts have no sign-in API yet), so the menu says exactly
 * that instead of inventing a user, e-mail or avatar.
 */
export function NavUser() {
  const { isMobile } = useSidebar()
  const { theme, setTheme } = useTheme()
  const [pending, startTransition] = useTransition()
  const vault = useDraftVault()
  const identity = (
    <>
      <Avatar className="rounded-lg after:rounded-lg">
        <AvatarFallback className="rounded-lg bg-primary text-[0.6875rem] font-semibold text-primary-foreground">管</AvatarFallback>
      </Avatar>
      <span className="grid flex-1 text-left text-sm leading-tight">
        <span className="truncate font-medium">管理员</span>
        <span className="truncate text-xs text-muted-foreground">{pending ? "正在退出…" : "管理员令牌会话"}</span>
      </span>
    </>
  )

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger
            aria-label="账户菜单"
            render={<SidebarMenuButton size="lg" className="aria-expanded:bg-sidebar-accent" />}
          >
            {identity}
            <ChevronsUpDownIcon aria-hidden="true" className="ml-auto size-4" />
          </DropdownMenuTrigger>
          <DropdownMenuContent className="w-60" side={isMobile ? "top" : "right"} align="end" sideOffset={4}>
            <DropdownMenuGroup>
              <DropdownMenuLabel className="p-0 font-normal">
                <span className="flex items-center gap-2 px-1 py-1.5 text-left text-foreground">{identity}</span>
              </DropdownMenuLabel>
              <p className="px-2 pb-1.5 text-xs text-muted-foreground">8 小时内有效；用户账户上线后显示登录用户。</p>
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              <DropdownMenuLabel>外观</DropdownMenuLabel>
              <DropdownMenuRadioGroup
                value={theme ?? "system"}
                onValueChange={(value) => {
                  if (typeof value === "string") setTheme(value)
                }}
              >
                {themes.map(({ value, label, icon: Icon }) => (
                  <DropdownMenuRadioItem key={value} value={value} closeOnClick>
                    <Icon />
                    {label}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem render={<a href={docsUrl} target="_blank" rel="noreferrer" />}>
              <BookOpenIcon />
              文档
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              disabled={pending}
              onClick={() => {
                // Unsaved drafts of every network belong to this session only.
                vault.reset()
                startTransition(() => logout())
              }}
            >
              <LogOutIcon />
              退出登录
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  )
}
