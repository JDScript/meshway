import type { NetworkSummary } from "@meshway/control-client"
import { useNavigate } from "@tanstack/react-router"
import { ChevronsUpDownIcon } from "lucide-react"
import { useState } from "react"

import { BrandMark } from "@/components/chrome/brand"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Skeleton } from "@/components/ui/skeleton"
import { CreateNetworkDialog } from "@/features/network/create-network"
import { NetworkMenuItems } from "@/features/network/network-menu"
import { roleLabel } from "@/features/network/roles"
import { m } from "@/paraglide/messages.js"

/**
 * Top left on every page of a network: which network, the switcher to the
 * others (and to a new one), and this account's role in it. Every slot is
 * always present, so reading the network never changes its size.
 */
export function NetworkChip({
  networkId,
  network,
  onSwitchNetwork,
}: {
  networkId: string
  /** null until read. */
  network: NetworkSummary | null
  onSwitchNetwork: (networkId: string) => void
}) {
  const navigate = useNavigate()
  const [creating, setCreating] = useState(false)

  return (
    // Concentric: the 32px switcher sits 6px in (22 − 6 = 16); the far side matches the 14px to the mark.
    <div className="glass pointer-events-auto flex h-(--bar-height) max-w-full min-w-0 items-center gap-2 rounded-full pr-3.5 pl-1.5 max-md:w-full md:gap-3">
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <button
              type="button"
              aria-label={m.networks_switch()}
              className="flex h-8 min-w-0 shrink items-center gap-2.5 rounded-full pr-2.5 pl-2 transition-colors outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring data-popup-open:bg-fill-selected"
            />
          }
        >
          {/* Narrow screens give the name the room. */}
          <BrandMark className="hidden size-5 shrink-0 md:block" />
          {network ? (
            <span className="max-w-60 min-w-0 truncate text-row font-medium">
              {network.name}
            </span>
          ) : (
            <Skeleton className="h-3 w-24 rounded-full" />
          )}
          <ChevronsUpDownIcon className="size-3.5 shrink-0 text-muted-foreground" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" sideOffset={8} className="min-w-60">
          <NetworkMenuItems
            current={networkId}
            onSwitch={onSwitchNetwork}
            onCreate={() => setCreating(true)}
          />
        </DropdownMenuContent>
      </DropdownMenu>
      <span
        aria-hidden="true"
        className="ml-auto h-4 w-px shrink-0 bg-divider"
      />
      <span className="flex h-5 shrink-0 items-center text-meta text-muted-foreground">
        {network ? (
          roleLabel[network.role]()
        ) : (
          <Skeleton className="h-2.5 w-10 rounded-full" />
        )}
      </span>
      <CreateNetworkDialog
        open={creating}
        onOpenChange={setCreating}
        onCreated={(created) =>
          void navigate({
            to: "/networks/$network/overview",
            params: { network: created },
          })
        }
      />
    </div>
  )
}
