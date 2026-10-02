import { ChevronsUpDownIcon } from "lucide-react"

import { BrandMark } from "@/components/chrome/brand"
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { NetworkMenuItems } from "@/features/network/network-menu"
import type { LinkLook } from "@/features/globe/scene"
import { m } from "@/paraglide/messages.js"

import { lookColor, lookLabel } from "./labels"
import type { LinkSummary } from "./model"

const DASH = "—"

/**
 * Top left: which network, and how it is doing at a glance. Every slot is
 * always present, so reads, failures and refreshes never change its size.
 */
export function NetworkChip({
  networkId,
  name,
  nodes,
  links,
  stale,
}: {
  networkId: string
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
  const linkValue = known ? `${known.confirmed}/${known.total}` : links?.kind === "none" ? "0" : DASH

  return (
    <div className="glass pointer-events-auto flex h-11 items-center gap-3 rounded-full pr-4 pl-1.5">
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <button
              type="button"
              className="flex h-8 min-w-0 items-center gap-2.5 rounded-full pr-2.5 pl-2 transition-colors outline-none hover:bg-foreground/6 focus-visible:ring-2 focus-visible:ring-ring data-open:bg-foreground/6"
            />
          }
        >
          <BrandMark className="size-5 shrink-0" />
          <span className="max-w-[28vw] truncate text-sm font-medium">{name ?? "\u00a0"}</span>
          <ChevronsUpDownIcon className="size-3.5 shrink-0 text-muted-foreground" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" sideOffset={8} className="min-w-60">
          <NetworkMenuItems current={networkId} />
        </DropdownMenuContent>
      </DropdownMenu>
      <span aria-hidden="true" className="h-4 w-px shrink-0 bg-foreground/12" />
      <Stat value={nodes === null ? DASH : String(nodes)} label={m.stat_nodes({ count: nodes ?? 0 })} />
      <Stat value={linkValue} label={m.stat_links({ count: known?.total ?? 0 })} />
      <Tooltip>
        <TooltipTrigger
          render={
            <span
              tabIndex={0}
              aria-label={rows.map((row) => `${lookLabel[row.look]()} ${row.value}`).join(", ") || undefined}
              className="relative flex h-1.5 w-20 shrink-0 gap-0.5 overflow-hidden rounded-full bg-foreground/8 outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {rows.map((row) =>
                row.value > 0 ? (
                  <span
                    key={row.look}
                    className="h-full rounded-full transition-[flex-grow] duration-700"
                    style={{ flexGrow: row.value, background: lookColor[row.look] }}
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
                  <span className="size-1.5 rounded-full" style={{ background: lookColor[row.look] }} />
                  <span className="flex-1">{lookLabel[row.look]()}</span>
                  <span className="tabular-nums">{row.value}</span>
                </li>
              ))}
            </ul>
          </TooltipContent>
        ) : null}
      </Tooltip>
      <Tooltip>
        <TooltipTrigger
          render={
            <span
              aria-label={stale ? m.overview_refresh_failed({ time: stale }) : undefined}
              className={stale ? "size-1.5 shrink-0 rounded-full bg-warning" : "size-1.5 shrink-0 rounded-full bg-transparent"}
            />
          }
        />
        {stale ? <TooltipContent>{m.overview_refresh_failed({ time: stale })}</TooltipContent> : null}
      </Tooltip>
    </div>
  )
}

function Stat({ value, label }: { value: string; label: string }) {
  return (
    <span className="flex shrink-0 items-baseline gap-1">
      <span className="text-[15px] font-semibold tabular-nums">{value}</span>
      <span className="text-xs text-muted-foreground">{label}</span>
    </span>
  )
}
