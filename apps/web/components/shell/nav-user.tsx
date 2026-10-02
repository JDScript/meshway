"use client"

import { BookOpenIcon, ChevronsUpDownIcon, KeyRoundIcon, LogOutIcon, MonitorIcon, MoonIcon, SunIcon, UserPlusIcon, UserRoundIcon } from "lucide-react"
import Link from "next/link"
import { useTheme } from "next-themes"
import { useTransition } from "react"

import { signOutRendered } from "@/components/auth/sign-out"
import { useDraftVault, useSignIn } from "@/components/network/network-workspace"
import { useNetwork } from "@/components/shell/network-context"
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
import { roleIn, roleLabels, type ConsoleIdentity } from "@/lib/account-model"
import { docsUrl } from "@/lib/config"

const themes = [
  { value: "system", label: "跟随系统", icon: MonitorIcon },
  { value: "light", label: "浅色", icon: SunIcon },
  { value: "dark", label: "深色", icon: MoonIcon },
] as const

function describe(identity: ConsoleIdentity, networkId: string): { initial: string; title: string; detail: string; note: string } {
  if (identity.kind === "bootstrap") {
    return {
      initial: "管",
      title: "实例管理员令牌",
      detail: "初始化与恢复会话",
      note: "不是任何人的账户，不能批准设备。日常管理请用账户登录。",
    }
  }
  if (identity.kind === "user-unverified") {
    return { initial: "?", title: "账户", detail: "暂时无法读取账户", note: identity.error }
  }
  const role = roleIn(identity, networkId)
  return {
    initial: identity.email.slice(0, 1).toUpperCase(),
    title: identity.email,
    detail: role ? roleLabels[role] : "不是本网络成员",
    note: "",
  }
}

/**
 * sidebar-07 `NavUser`: who is actually signed in (a user account and its
 * role in this network, or the administrator token), read from the
 * controller for this render. Nothing here is invented.
 */
export function NavUser({ identity }: { identity: ConsoleIdentity }) {
  const { isMobile } = useSidebar()
  const { theme, setTheme } = useTheme()
  const [pending, startTransition] = useTransition()
  const vault = useDraftVault()
  const signIn = useSignIn()
  const { networkId, routes } = useNetwork()
  const view = describe(identity, networkId)
  const role = roleIn(identity, networkId)
  const identityRow = (
    <>
      <Avatar className="rounded-lg after:rounded-lg">
        <AvatarFallback className="rounded-lg bg-primary text-[0.6875rem] font-semibold text-primary-foreground">{view.initial}</AvatarFallback>
      </Avatar>
      <span className="grid flex-1 text-left text-sm leading-tight">
        <span className="truncate font-medium">{view.title}</span>
        <span className="truncate text-xs text-muted-foreground">{pending ? "正在退出…" : view.detail}</span>
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
            {identityRow}
            <ChevronsUpDownIcon aria-hidden="true" className="ml-auto size-4" />
          </DropdownMenuTrigger>
          <DropdownMenuContent className="w-60" side={isMobile ? "top" : "right"} align="end" sideOffset={4}>
            <DropdownMenuGroup>
              <DropdownMenuLabel className="p-0 font-normal">
                <span className="flex items-center gap-2 px-1 py-1.5 text-left text-foreground">{identityRow}</span>
              </DropdownMenuLabel>
              {view.note ? <p className="px-2 pb-1.5 text-xs break-words text-muted-foreground">{view.note}</p> : null}
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {identity.kind === "user" ? (
                <DropdownMenuItem render={<Link href={routes.account} />}>
                  <UserRoundIcon />
                  我的账户
                </DropdownMenuItem>
              ) : null}
              {identity.kind === "bootstrap" || role === "owner" || role === "admin" ? (
                <DropdownMenuItem render={<Link href={`${routes.members}?invite=1`} />}>
                  <UserPlusIcon />
                  邀请成员
                </DropdownMenuItem>
              ) : null}
              {identity.kind === "bootstrap" ? (
                <DropdownMenuItem render={<Link href="/setup" />}>
                  <KeyRoundIcon />
                  实例初始化与恢复
                </DropdownMenuItem>
              ) : null}
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
                startTransition(() => signOutRendered(signIn))
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
