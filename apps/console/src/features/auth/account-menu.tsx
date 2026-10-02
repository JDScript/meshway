import { useQuery } from "@tanstack/react-query"
import { LogOutIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { sessionQuery } from "@/features/network/queries"
import { m } from "@/paraglide/messages.js"

import { useSignOut } from "./use-sign-out"

export function AccountMenu() {
  const session = useQuery(sessionQuery())
  const signOut = useSignOut()
  const email = session.data?.user.email ?? null
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button variant="ghost" size="icon" aria-label={m.account_menu()} className="rounded-full">
            <span className="flex size-7 items-center justify-center rounded-full bg-foreground text-xs font-semibold text-background uppercase">
              {email?.[0] ?? "·"}
            </span>
          </Button>
        }
      />
      <DropdownMenuContent align="end" className="min-w-56">
        <DropdownMenuGroup>
          <DropdownMenuLabel className="truncate font-normal text-muted-foreground">{email ?? m.account_menu()}</DropdownMenuLabel>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={() => void signOut()}>
          <LogOutIcon />
          {m.sign_out()}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
