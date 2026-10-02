import { useQuery } from "@tanstack/react-query"
import { useNavigate, useRouterState } from "@tanstack/react-router"
import { CheckIcon } from "lucide-react"

import { DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel } from "@/components/ui/dropdown-menu"
import { m } from "@/paraglide/messages.js"

import { networksQuery } from "./queries"

/** Switching keeps the page (overview stays overview, nodes stays nodes) but drops its selection. */
export function useSwitchNetwork(current: string) {
  const navigate = useNavigate()
  const pathname = useRouterState({ select: (state) => state.location.pathname })
  return (networkId: string) => {
    if (networkId === current) return
    void navigate({ href: pathname.replace(/^\/networks\/[0-9a-f]{32}/, `/networks/${networkId}`) })
  }
}

/** The account's networks, for any dropdown that switches between them. */
export function NetworkMenuItems({ current }: { current: string }) {
  const networks = useQuery(networksQuery())
  const switchTo = useSwitchNetwork(current)
  return (
    <DropdownMenuGroup>
      <DropdownMenuLabel className="text-xs text-muted-foreground">{m.networks_label()}</DropdownMenuLabel>
      {(networks.data?.networks ?? []).map((network) => (
        <DropdownMenuItem key={network.network_id} onClick={() => switchTo(network.network_id)} className="gap-2.5 py-2">
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate">{network.name}</span>
            <span className="font-mono text-[11px] text-muted-foreground">{network.network.ipv4_prefix}</span>
          </span>
          <CheckIcon className={network.network_id === current ? "size-4" : "size-4 opacity-0"} />
        </DropdownMenuItem>
      ))}
    </DropdownMenuGroup>
  )
}
