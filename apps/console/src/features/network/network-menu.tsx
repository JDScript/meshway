import { useQuery } from "@tanstack/react-query"
import { CheckIcon } from "lucide-react"

import { DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel } from "@/components/ui/dropdown-menu"
import { m } from "@/paraglide/messages.js"

import { networksQuery } from "./queries"

/** The account's networks, for any dropdown that switches between them. */
export function NetworkMenuItems({ current, onSwitch }: { current: string; onSwitch: (networkId: string) => void }) {
  const networks = useQuery(networksQuery())
  return (
    <DropdownMenuGroup>
      <DropdownMenuLabel className="text-xs text-muted-foreground">{m.networks_label()}</DropdownMenuLabel>
      {(networks.data?.networks ?? []).map((network) => (
        <DropdownMenuItem key={network.network_id} onClick={() => onSwitch(network.network_id)} className="gap-2.5 py-2">
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate">{network.name}</span>
            <span className="font-mono text-meta text-muted-foreground">{network.network.ipv4_prefix}</span>
          </span>
          <CheckIcon className={network.network_id === current ? "size-4" : "size-4 opacity-0"} />
        </DropdownMenuItem>
      ))}
    </DropdownMenuGroup>
  )
}
