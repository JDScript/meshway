import { useQuery } from "@tanstack/react-query"
import { ChevronsUpDownIcon, LanguagesIcon, LogOutIcon, MoonIcon, SunIcon } from "lucide-react"

import { useTheme } from "@/components/theme-provider"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem, useSidebar } from "@/components/ui/sidebar"
import { useSignOut } from "@/features/auth/use-sign-out"
import { sessionQuery } from "@/features/network/queries"
import { m } from "@/paraglide/messages.js"
import { getLocale, isLocale, locales, setLocale } from "@/paraglide/runtime.js"

const roleLabel = { owner: () => m.role_owner(), admin: () => m.role_admin(), member: () => m.role_member() }

function Initial({ email }: { email: string | null }) {
  return (
    <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-sidebar-primary text-xs font-semibold text-sidebar-primary-foreground uppercase">
      {email?.[0] ?? "·"}
    </span>
  )
}

/** sidebar-07's user menu: the signed-in account, its role here, preferences and sign-out. */
export function NavUser({ networkId }: { networkId: string }) {
  const { isMobile } = useSidebar()
  const session = useQuery(sessionQuery())
  const signOut = useSignOut()
  const { theme, setTheme } = useTheme()
  const email = session.data?.user.email ?? null
  const role = session.data?.memberships.find((membership) => membership.network_id === networkId)?.role
  const dark = theme === "dark" || (theme === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches)

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger render={<SidebarMenuButton size="lg" className="aria-expanded:bg-muted" />}>
            <Initial email={email} />
            <div className="grid flex-1 text-left text-sm leading-tight">
              <span className="truncate font-medium">{email ?? m.account_menu()}</span>
              <span className="truncate text-xs text-muted-foreground">{role ? roleLabel[role]() : "\u00a0"}</span>
            </div>
            <ChevronsUpDownIcon className="ml-auto size-4" />
          </DropdownMenuTrigger>
          <DropdownMenuContent className="min-w-56" side={isMobile ? "bottom" : "right"} align="end" sideOffset={4}>
            <DropdownMenuGroup>
              <DropdownMenuLabel className="truncate font-normal text-muted-foreground">{email ?? m.account_menu()}</DropdownMenuLabel>
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              <DropdownMenuItem onClick={() => setTheme(dark ? "light" : "dark")}>
                {dark ? <SunIcon /> : <MoonIcon />}
                {m.theme_switch()}
              </DropdownMenuItem>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <LanguagesIcon />
                  {m.language_switch()}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent>
                  <DropdownMenuRadioGroup
                    value={getLocale()}
                    onValueChange={(value) => {
                      if (typeof value === "string" && isLocale(value) && value !== getLocale()) setLocale(value)
                    }}
                  >
                    {locales.map((locale) => (
                      <DropdownMenuRadioItem key={locale} value={locale}>
                        {m.language_name({}, { locale })}
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={() => void signOut()}>
              <LogOutIcon />
              {m.sign_out()}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  )
}
