import { useInfiniteQuery } from "@tanstack/react-query"
import { CheckIcon, PlusIcon } from "lucide-react"

import { ApiFailure, problemCode } from "@/api/client"
import { failureText, toastFailure } from "@/api/failure"
import { networksQuery } from "@/api/queries/network"
import {
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { roleLabel } from "@/lib/roles"
import { m } from "@/paraglide/messages.js"

/**
 * The account's networks, for the dropdown that switches between them, a
 * page at a time, then creating another.
 */
export function NetworkMenuItems({
  current,
  onSwitch,
  onCreate,
}: {
  current: string
  onSwitch: (networkId: string) => void
  onCreate: () => void
}) {
  const networks = useInfiniteQuery(networksQuery())
  const items = networks.data?.pages.flatMap((page) => page.items) ?? []

  const loadMore = () =>
    void networks.fetchNextPage().then((result) => {
      if (!result.isFetchNextPageError) return
      // An expired cursor: read the list again from its start.
      if (
        result.error instanceof ApiFailure &&
        problemCode(result.error.failure) === "invalid_cursor"
      )
        void networks.refetch()
      else toastFailure(result.error)
    })

  return (
    <>
      <DropdownMenuGroup>
        <DropdownMenuLabel>{m.networks_label()}</DropdownMenuLabel>
        {networks.isPending ? (
          <div className="flex h-12 items-center px-2">
            <Skeleton className="h-3 w-32 rounded-full" />
          </div>
        ) : null}
        {networks.isError && items.length === 0 ? (
          <DropdownMenuItem
            closeOnClick={false}
            onClick={() => void networks.refetch()}
            className="py-2"
          >
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate text-destructive">
                {failureText(networks.error) ?? m.networks_failed()}
              </span>
              <span className="text-meta text-muted-foreground">
                {m.retry()}
              </span>
            </span>
          </DropdownMenuItem>
        ) : null}
        {items.map((network) => (
          <DropdownMenuItem
            key={network.id}
            onClick={() => onSwitch(network.id)}
            className="gap-2.5 py-2"
          >
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate">{network.name}</span>
              <span className="text-meta text-muted-foreground">
                {roleLabel[network.role]()}
              </span>
            </span>
            <CheckIcon
              className={network.id === current ? "size-4" : "size-4 opacity-0"}
            />
          </DropdownMenuItem>
        ))}
        {networks.hasNextPage ? (
          <DropdownMenuItem
            closeOnClick={false}
            disabled={networks.isFetchingNextPage}
            onClick={loadMore}
            className="text-muted-foreground"
          >
            {/* The glyph's slot is always there, so the label never moves. */}
            <span className="flex size-4 items-center justify-center">
              {networks.isFetchingNextPage ? <Spinner /> : null}
            </span>
            {m.networks_more()}
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuGroup>
      <DropdownMenuSeparator />
      <DropdownMenuItem onClick={onCreate}>
        <PlusIcon />
        {m.network_create()}
      </DropdownMenuItem>
    </>
  )
}
