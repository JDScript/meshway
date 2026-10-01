"use client"

import { BookOpenIcon, ChevronsUpDownIcon, LogOutIcon, MonitorIcon, MoonIcon, SunIcon } from "lucide-react"
import { useTheme } from "next-themes"
import { useTransition } from "react"

import { useDraftVault } from "@/components/network/network-workspace"
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
import { logout } from "@/lib/auth-actions"
import { docsUrl } from "@/lib/config"

const themes = [
  { value: "system", label: "跟随系统", icon: MonitorIcon },
  { value: "light", label: "浅色", icon: SunIcon },
  { value: "dark", label: "深色", icon: MoonIcon },
] as const

/**
 * The console has one administrator token session today; user accounts
 * arrive with local sign-in. The menu says so rather than inventing a user.
 */
export function AccountMenu() {
  const { theme, setTheme } = useTheme()
  const [pending, startTransition] = useTransition()
  const vault = useDraftVault()

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label="账户菜单"
        className="flex w-full min-w-0 items-center gap-2.5 rounded-md p-1 text-left outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-sidebar-ring aria-expanded:bg-sidebar-accent group-data-[collapsible=icon]:size-8 group-data-[collapsible=icon]:p-0.5"
      >
        <span
          aria-hidden="true"
          className="flex size-7 shrink-0 items-center justify-center rounded-full bg-primary text-[0.6875rem] font-semibold text-primary-foreground"
        >
          管
        </span>
        <span className="flex min-w-0 flex-1 flex-col group-data-[collapsible=icon]:hidden">
          <span className="truncate text-[0.8125rem] leading-5 font-medium">管理员</span>
          <span className="truncate text-[0.6875rem] leading-4 text-muted-foreground">{pending ? "正在退出…" : "管理员令牌会话"}</span>
        </span>
        <ChevronsUpDownIcon className="size-4 shrink-0 text-muted-foreground group-data-[collapsible=icon]:hidden" aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="start" sideOffset={6} className="w-60">
        <DropdownMenuGroup>
          <DropdownMenuLabel className="flex flex-col gap-0.5">
            <span className="text-[0.8125rem] font-medium text-foreground">管理员</span>
            <span className="font-normal">{pending ? "正在退出…" : "管理员令牌会话 · 8 小时内有效"}</span>
          </DropdownMenuLabel>
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
  )
}
