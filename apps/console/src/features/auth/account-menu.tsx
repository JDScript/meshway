import { useQuery } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import { LogOutIcon, UserRoundIcon } from "lucide-react"

import { InitialAvatar } from "@/components/chrome/avatar"
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
import { useAccountNavigation } from "@/features/account/navigation"
import { sessionQuery } from "@/features/network/queries"
import { m } from "@/paraglide/messages.js"

import { useSignOut } from "./use-sign-out"

export function AccountMenu() {
  const session = useQuery(sessionQuery())
  const networkId = useParams({
    strict: false,
    select: (params) => params.network ?? null,
  })
  const signOut = useSignOut()
  const email = session.data?.user.email ?? null
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            aria-label={m.account_menu()}
            className="bar-button"
          >
            <InitialAvatar email={email} size="md" tone="inverse" />
          </Button>
        }
      />
      <DropdownMenuContent align="end" className="min-w-56">
        <DropdownMenuGroup>
          <DropdownMenuLabel className="truncate font-normal text-muted-foreground">
            {email ?? m.account_menu()}
          </DropdownMenuLabel>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        {networkId ? <AccountItem networkId={networkId} /> : null}
        <DropdownMenuItem onClick={() => void signOut()}>
          <LogOutIcon />
          {m.sign_out()}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** The account page, opened over the network's current page. */
function AccountItem({ networkId }: { networkId: string }) {
  const account = useAccountNavigation(networkId)
  return (
    <DropdownMenuItem onClick={account.open}>
      <UserRoundIcon />
      {m.account_title()}
    </DropdownMenuItem>
  )
}
