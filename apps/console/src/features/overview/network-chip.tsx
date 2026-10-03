import { ChevronsUpDownIcon } from "lucide-react"
import type { ReactNode } from "react"

import { BrandMark } from "@/components/chrome/brand"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { LinkLook } from "@/features/globe/scene"
import { NetworkMenuItems } from "@/features/network/network-menu"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { lookColor, lookLabel } from "./labels"
import type { LinkSummary } from "./model"
import { LookDot } from "./status-dots"

const DASH = "—"

/**
 * Top left: which network, and how it is doing at a glance. Every slot is
 * always present, so reads, failures and refreshes never change its size.
 */
export function NetworkChip({
  networkId,
  onSwitchNetwork,
  trailing,
  name,
  nodes,
  links,
  stale,
}: {
  networkId: string
  onSwitchNetwork: (networkId: string) => void
  /** A control at the card's far end, set 6px in like the switcher (the unplaced count). */
  trailing?: ReactNode
  name: string | null
  /** null while unread or unreadable. */
  nodes: number | null
  links: LinkSummary | null
  /** Time of the last good runtime read when the latest refresh failed. */
  stale: string | null
}) {
  const known = links?.kind === "known" ? links : null
  const rows: { look: LinkLook; value: number }[] = known
    ? [
        { look: "confirmed", value: known.confirmed },
        { look: "one_sided", value: known.oneSided },
        { look: "degraded", value: known.degraded },
        { look: "configured", value: known.unreported },
      ]
    : []
  const linkValue = known
    ? `${known.confirmed}/${known.total}`
    : links?.kind === "none"
      ? "0"
      : DASH

  return (
    // Concentric: the 32px controls at either end sit 6px in (22 − 6 = 16); without one, the far side matches the 14px to the mark.
    <div
      className={cn(
        // On phones it fills the bottom bar beside the launcher: the name left, the counts right.
        "glass pointer-events-auto flex h-(--bar-height) max-w-full min-w-0 items-center gap-2 rounded-full pl-1.5 max-md:w-full md:gap-3",
        trailing ? "pr-1.5" : "pr-3.5"
      )}
    >
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <button
              type="button"
              className="flex h-8 min-w-0 shrink items-center gap-2.5 rounded-full pr-2.5 pl-2 transition-colors outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring data-popup-open:bg-fill-selected"
            />
          }
        >
          {/* Narrow screens give the name the room. */}
          <BrandMark className="hidden size-5 shrink-0 md:block" />
          <span className="max-w-60 min-w-0 truncate text-row font-medium">
            {name ?? "\u00a0"}
          </span>
          <ChevronsUpDownIcon className="size-3.5 shrink-0 text-muted-foreground" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" sideOffset={8} className="min-w-60">
          <NetworkMenuItems current={networkId} onSwitch={onSwitchNetwork} />
        </DropdownMenuContent>
      </DropdownMenu>
      <span
        aria-hidden="true"
        className="ml-auto h-4 w-px shrink-0 bg-divider"
      />
      <Stat
        value={nodes === null ? DASH : String(nodes)}
        label={m.stat_nodes({ count: nodes ?? 0 })}
      />
      <Stat
        value={linkValue}
        label={m.stat_links({ count: known?.total ?? 0 })}
      />
      {/*
        The stale mark overlays the bar's end, so nothing is reserved for it and nothing moves when it appears.
        Phones leave the bar out (its breakdown is a hover; the count beside it says the share, and the name needs the room) but keep the stale mark, right after the count.
      */}
      <span className="relative flex shrink-0 max-md:-ml-2 max-md:w-0">
        <Tooltip>
          <TooltipTrigger
            render={
              <span
                tabIndex={0}
                aria-label={
                  rows
                    .map((row) => `${lookLabel[row.look]()} ${row.value}`)
                    .join(", ") || undefined
                }
                className="relative flex h-1.5 w-20 gap-0.5 overflow-hidden rounded-full bg-fill-selected outline-none focus-visible:ring-2 focus-visible:ring-ring max-md:hidden"
              >
                {rows.map((row) =>
                  row.value > 0 ? (
                    <span
                      key={row.look}
                      className="h-full rounded-full transition-[flex-grow] duration-700"
                      style={{
                        flexGrow: row.value,
                        background: lookColor[row.look],
                      }}
                    />
                  ) : null
                )}
              </span>
            }
          />
          {rows.length > 0 ? (
            <TooltipContent className="min-w-40">
              <ul className="flex flex-col gap-1.5 py-0.5">
                {rows.map((row) => (
                  <li key={row.look} className="flex items-center gap-2.5">
                    <LookDot look={row.look} size="sm" />
                    <span className="flex-1">{lookLabel[row.look]()}</span>
                    <span className="tabular-nums">{row.value}</span>
                  </li>
                ))}
              </ul>
            </TooltipContent>
          ) : null}
        </Tooltip>
        {stale ? (
          <Tooltip>
            <TooltipTrigger
              render={
                <span
                  aria-label={m.overview_refresh_failed({ time: stale })}
                  className="absolute top-1/2 -right-2.5 size-1.5 -translate-y-1/2 rounded-full bg-warning max-md:-right-1.75"
                />
              }
            />
            <TooltipContent>
              {m.overview_refresh_failed({ time: stale })}
            </TooltipContent>
          </Tooltip>
        ) : null}
      </span>
      {trailing}
    </div>
  )
}

function Stat({ value, label }: { value: string; label: string }) {
  return (
    <span className="flex shrink-0 items-baseline gap-1">
      <span className="text-lead font-semibold tabular-nums">{value}</span>
      <span className="text-meta text-muted-foreground">{label}</span>
    </span>
  )
}
