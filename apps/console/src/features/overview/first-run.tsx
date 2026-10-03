import { useMatch } from "@tanstack/react-router"
import { PlusIcon } from "lucide-react"
import type { CSSProperties } from "react"

import { Button } from "@/components/ui/button"
import { useBootStore } from "@/features/boot/boot-store"
import { useCanManage } from "@/features/network/role"
import { useNetwork } from "@/features/network/use-network-view"
import { useNodeNavigation } from "@/features/nodes/navigation"
import {
  EDGE,
  GAP,
  MOBILE_EDGE,
  MOBILE_SHEET_BOTTOM,
  PEEK_WIDTH,
} from "@/features/shell/layout"
import { Surface } from "@/features/shell/surface"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

const OVERVIEW = "/_authed/networks/$network/overview" as const
/** The place at the bottom; it follows the peek as the globe does. */
const SLOT =
  "pointer-events-none absolute flex justify-center transition-[right] duration-500 ease-settle"

/**
 * The overview of a network with no node, or with one: what adding nodes
 * takes, over the globe at the bottom (above the bottom bar on phones),
 * clear of the peek. Only once the inventory has been read: loading and
 * failures are never taken for an empty network. It cannot be dismissed;
 * it goes when the network has its nodes.
 */
export function FirstRun({
  networkId,
  mobile,
  peek,
}: {
  networkId: string
  mobile: boolean
  /** The peek is open: on phones its sheet takes this place. */
  peek: boolean
}) {
  const view = useNetwork()
  const nav = useNodeNavigation(networkId)
  const canManage = useCanManage(networkId)
  const ready = useBootStore((store) => store.phase === "ready")
  const current =
    useMatch({ from: OVERVIEW, shouldThrow: false, select: () => true }) ??
    false
  const count = view.nodes.data
    ? view.inventory.filter((node) => node.revoked_at === null).length
    : null
  const shown = ready && current && !(mobile && peek)

  // Centred in the room the peek leaves; on phones between the edges, above the bottom bar.
  const place: CSSProperties = mobile
    ? { left: MOBILE_EDGE, right: MOBILE_EDGE, bottom: MOBILE_SHEET_BOTTOM }
    : { left: 0, right: peek ? PEEK_WIDTH + EDGE + GAP : 0, bottom: EDGE }
  // In the one-line chip on phones the button is its glyph (its name stays for assistive tech), so the fact keeps the line.
  const add = (compact: boolean) =>
    canManage ? (
      <Button
        className={cn(
          "h-9 shrink-0 rounded-full px-4",
          compact && "max-md:h-8 max-md:w-8 max-md:px-0"
        )}
        onClick={nav.openAdd}
      >
        <PlusIcon />
        <span className={cn(compact && "max-md:sr-only")}>
          {m.nodes_add_title()}
        </span>
      </Button>
    ) : null

  return (
    <>
      <div className={SLOT} style={place}>
        <Surface
          shown={shown && count === 0}
          from="bottom"
          aria-labelledby="first-run-title"
          className="glass pointer-events-auto flex w-full max-w-md flex-col gap-4 rounded-surface p-5"
        >
          <h2 id="first-run-title" className="text-lead font-semibold">
            {m.overview_first_title()}
          </h2>
          <ol className="flex flex-col gap-2.5 text-row">
            {[
              m.overview_first_token(),
              m.overview_first_run(),
              m.overview_first_appear(),
            ].map((fact, index) => (
              <li key={fact} className="flex items-baseline gap-3">
                <span
                  aria-hidden="true"
                  className="flex size-5 shrink-0 translate-y-px items-center justify-center self-start rounded-full bg-fill-selected text-meta font-medium tabular-nums"
                >
                  {index + 1}
                </span>
                <span className="min-w-0">{fact}</span>
              </li>
            ))}
          </ol>
          {canManage ? (
            <div className="flex justify-end">{add(false)}</div>
          ) : (
            <p className="text-meta text-muted-foreground">
              {m.overview_first_admins()}
            </p>
          )}
        </Surface>
      </div>
      <div className={SLOT} style={place}>
        <Surface
          shown={shown && count === 1}
          from="bottom"
          className={cn(
            "glass pointer-events-auto flex h-(--bar-height) max-w-full items-center gap-3 rounded-full pl-4 max-md:w-full",
            canManage ? "pr-1.5" : "pr-4"
          )}
        >
          <p className="min-w-0 flex-1 truncate text-row">
            {m.overview_second()}
          </p>
          {add(true)}
        </Surface>
      </div>
    </>
  )
}
